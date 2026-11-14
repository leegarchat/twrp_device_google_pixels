/*
 * tcpm_drp_shim.c — DRP promotion for sink-clobbered TCPC ports (stock kernels).
 *
 * Some bootloaders bring a dual-role connector up sink-only for recovery
 * and fastbootd, which leaves the tcpm port fixed at SNK: every
 * `power_role=source` write then fails with EOPNOTSUPP and VBUS never
 * comes up. The port still lists source-pdos, so the condition
 * (SNK + source-pdos present) identifies exactly the clobbered ports —
 * genuine sink-only ports carry no source-pdos and are untouched, as
 * are normal boots that present dual-role directly. This is the same
 * condition as asdfmonster261's tcpm patch (espada ec0cf24c4 /
 * heybooboo d1ce67336), applied at runtime instead of compile time.
 *
 * Mechanism (kretprobe on `typec_get_fw_cap`, the DT-capability reader
 * tcpm calls first in `tcpm_fw_get_caps`): at return the capability
 * block is fully populated, so the return handler checks the condition
 * and promotes the block in place (type=DRP, data=DRD,
 * prefer_role=SINK). `tcpm_fw_get_caps` then assigns
 * `port->port_type` from the already-promoted block, so the whole
 * downstream probe — including the typec class registration — sees DRP
 * with zero knowledge of this module. Byte-identical outcome to the
 * compiled-in patch.
 *
 * Why this hook and not `tcpm_fw_get_caps` itself: `struct tcpm_port`
 * is private to tcpm.c (no KMI, layout drifts between trees), while
 * `struct typec_capability` is public (linux/usb/typec.h, KMI-guarded
 * by ANDROID_KABI_RESERVE) — the module touches ONLY the public
 * struct, so one build covers every 6.12 patch level. A three-field
 * canary (type/data in range) plus the condition itself fail closed:
 * on any layout drift the values cannot all read sane, and nothing is
 * written.
 *
 * Probe-time only: tcpm reads the caps once per probe, so after arming
 * the shim the TCPC must re-probe (recovery-pixel-boot unbinds/binds
 * the TCPC on stock sink ports when entering host mode with the drive
 * intent and nothing mounted). No re-probe, no patch — the module
 * never touches a live port.
 *
 * No conflict with DRP-patched custom kernels: there the caps already
 * read DRP, the condition never matches, zero writes. The daemon
 * additionally skips the re-probe when the port already reads dual.
 * Laguna shares nothing with this module (different vote, different
 * load path); malibu-only by daemon wiring.
 *
 * Symbols are resolved with kprobes (same "Kprobe Edition" design as
 * aoc_vote_shim): no link-time dependency, builds against any tree.
 * fwnode_property_present is core API with a stable prototype (copied
 * exactly from include/linux/property.h — KCFI hashes types, so the
 * declaration must stay verbatim).
 *
 * Proc interfaces (mirror aoc_vote_shim):
 * /proc/tcpm_drp_shim — write "1"=arm, "0"=disarm; read returns stats
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/types.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/seq_file.h>
#include <linux/kprobes.h>
#include <linux/ptrace.h>
#include <linux/compiler.h>
#include <linux/atomic.h>
#include <linux/err.h>
#include <linux/string.h>
#include <linux/usb/typec.h>
#include <linux/fwnode.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("tcpm DRP promotion shim for sink-clobbered ports (stock-kernel OTG)");

/* fwnode_property_present (include/linux/property.h). Resolved at
 * runtime via kprobe — see file header. The declaration below MUST
 * match the true prototype exactly (KCFI). */
static bool (*real_fwnode_present)(const struct fwnode_handle *fwnode,
				   const char *propname) = NULL;

static int resolve_fwnode_symbol(void)
{
	struct kprobe kp;
	int ret;

	if (real_fwnode_present)
		return 0;
	memset(&kp, 0, sizeof(kp));
	kp.symbol_name = "fwnode_property_present";
	ret = register_kprobe(&kp);
	if (ret < 0) {
		pr_err("tcpm_drp_shim: symbol 'fwnode_property_present' not found (err=%d)\n",
		       ret);
		return ret;
	}
	real_fwnode_present = (bool (*)(const struct fwnode_handle *,
					const char *))kp.addr;
	unregister_kprobe(&kp);

	if (!real_fwnode_present) {
		pr_err("tcpm_drp_shim: kprobe succeeded but address is NULL\n");
		return -ENOENT;
	}
	pr_info("tcpm_drp_shim: resolved fwnode_property_present at %p\n",
		real_fwnode_present);
	return 0;
}

/* One kretprobe instance carries the entry args to the return handler.
 * fwnode is captured at entry (x1): some trees only assign cap->fwnode
 * later in the probe, so the entry value is primary, cap->fwnode the
 * fallback. */
struct probe_slot {
	struct typec_capability *cap;
	struct fwnode_handle *fwnode;
};

static bool armed;
static atomic_t patched;
static struct proc_dir_entry *proc_shim;

static int fwcap_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct probe_slot *s = (struct probe_slot *)ri->data;

	if (!READ_ONCE(armed))
		return 1; /* skip the return probe while disarmed */
	/* AAPCS64: arg0 (cap) arrives in x0, arg1 (fwnode) in x1. */
	s->cap = (struct typec_capability *)regs->regs[0];
	s->fwnode = (struct fwnode_handle *)regs->regs[1];
	return 0;
}

static int fwcap_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct probe_slot *s = (struct probe_slot *)ri->data;
	struct typec_capability *cap;
	struct fwnode_handle *fwnode;

	if (!READ_ONCE(armed))
		return 0;
	cap = s->cap;
	if (!cap)
		return 0;
	if (regs_return_value(regs) != 0)
		return 0; /* DT read failed; leave the port alone */
	/* Canary: a sane, fully-populated capability block. Any layout
	 * drift fails here before a single byte is written. */
	if (cap->type > TYPEC_PORT_DRP || cap->data > TYPEC_PORT_DRD)
		return 0;
	if (cap->type != TYPEC_PORT_SNK)
		return 0; /* DRP boots and genuine sinks untouched */
	fwnode = s->fwnode ? s->fwnode : cap->fwnode;
	if (!fwnode)
		return 0;
	/* Context note: the return handler runs in a debug exception with
	 * IRQs off. Only the DT (of_fwnode, raw_spin_lock_irqsave) flavor
	 * of fwnode_property_present is safe here — never add a sleeping
	 * fwnode_* helper to this path. Pixel TCPC fwnodes are DT. */
	if (!real_fwnode_present || !real_fwnode_present(fwnode, "source-pdos"))
		return 0; /* genuinely sink-only: no source PDOs */
	cap->type = TYPEC_PORT_DRP;
	cap->data = TYPEC_PORT_DRD;
	cap->prefer_role = TYPEC_SINK;
	pr_info("tcpm_drp_shim: sink-clobbered port promoted to DRP (patch #%d)\n",
		atomic_inc_return(&patched));
	return 0;
}

static struct kretprobe fwcap_probe = {
	.kp = {
		.symbol_name = "typec_get_fw_cap",
	},
	.entry_handler = fwcap_entry,
	.handler = fwcap_ret,
	.data_size = sizeof(struct probe_slot),
	.maxactive = 4,
};

static ssize_t shim_write(struct file *file, const char __user *ubuf,
			  size_t count, loff_t *ppos)
{
	char buf[4];
	bool on;

	if (count == 0 || count > sizeof(buf) - 1)
		return -EINVAL;

	if (copy_from_user(buf, ubuf, count))
		return -EFAULT;
	buf[count] = '\0';

	if (buf[0] == '1')
		on = true;
	else if (buf[0] == '0')
		on = false;
	else
		return -EINVAL;

	WRITE_ONCE(armed, on);
	pr_info("tcpm_drp_shim: %s\n", on ? "armed" : "disarmed");
	return count;
}

static int shim_show(struct seq_file *m, void *v)
{
	seq_printf(m, "armed=%d patched=%d\n", READ_ONCE(armed) ? 1 : 0,
		   atomic_read(&patched));
	return 0;
}

static int shim_open(struct inode *inode, struct file *file)
{
	return single_open(file, shim_show, NULL);
}

static const struct proc_ops shim_ops = {
	.proc_open    = shim_open,
	.proc_read    = seq_read,
	.proc_write   = shim_write,
	.proc_lseek   = seq_lseek,
	.proc_release = single_release,
};

static int __init tcpm_drp_shim_init(void)
{
	int ret;

	ret = resolve_fwnode_symbol();
	if (ret)
		return ret;

	ret = register_kretprobe(&fwcap_probe);
	if (ret < 0) {
		pr_err("tcpm_drp_shim: kretprobe on 'typec_get_fw_cap' failed (err=%d, inlined?)\n",
		       ret);
		return ret;
	}

	proc_shim = proc_create("tcpm_drp_shim", 0660, NULL, &shim_ops);
	if (!proc_shim) {
		pr_err("tcpm_drp_shim: failed to create /proc/tcpm_drp_shim\n");
		unregister_kretprobe(&fwcap_probe);
		return -ENOMEM;
	}

	WRITE_ONCE(armed, true);
	pr_info("tcpm_drp_shim: loaded and armed (hook on typec_get_fw_cap)\n");
	return 0;
}

static void __exit tcpm_drp_shim_exit(void)
{
	WRITE_ONCE(armed, false);
	if (proc_shim)
		proc_remove(proc_shim);
	unregister_kretprobe(&fwcap_probe);
	pr_info("tcpm_drp_shim: unloaded (patched %d ports)\n", atomic_read(&patched));
}

module_init(tcpm_drp_shim_init);
module_exit(tcpm_drp_shim_exit);
