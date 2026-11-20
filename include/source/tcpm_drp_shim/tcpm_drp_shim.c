/*
 * tcpm_drp_shim.c — Live + probe-time DRP promotion for sink-clobbered TCPC ports.
 *
 * Bootloaders on Tensor G6 (malibu) bring the dual-role USB-C connector up
 * as "sink" in recovery/fastbootd, leaving tcpm fixed at TYPEC_PORT_SNK.
 *
 * Two mechanisms in one module:
 * 1. Live promotion (command "1" or "2" to /proc/tcpm_drp_shim, and auto at
 *    module_init): locates /sys/class/typec/port0 via typec_class, finds its
 *    heap-allocated struct typec_capability (matched by cap->fwnode ==
 *    dev->fwnode && cap->ops == port->ops), promotes cap->type = TYPEC_PORT_DRP
 *    and cap->data = TYPEC_PORT_DRD, updates port->port_type = TYPEC_PORT_DRP,
 *    and invokes port->ops->port_type_set(port, TYPEC_PORT_DRP)
 *    (tcpm_port_type_set), which sets tcpm->port_type = TYPEC_PORT_DRP and
 *    transitions the state machine through PORT_RESET -> DRP toggling.
 *    No driver unbind/re-probe required.
 * 2. Probe-time kretprobe on typec_get_fw_cap: if the TCPC probes after us,
 *    promotes the capability block in place before typec_register_port runs.
 *
 * Safe on custom kernels with the compiled-in DRP patch (Anvil/Espada):
 * if the port is already TYPEC_PORT_DRP, both paths are pure no-ops.
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
#include <linux/device.h>
#include <linux/device/class.h>
#include <linux/device/bus.h>
#include <linux/err.h>
#include <linux/string.h>
#include <linux/usb/typec.h>
#include <linux/fwnode.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("tcpm live + probe-time DRP promotion shim for recovery OTG");

static bool (*real_fwnode_present)(const struct fwnode_handle *fwnode,
				   const char *propname) = NULL;
static unsigned long (*real_kallsyms_lookup_name)(const char *name) = NULL;

static int resolve_symbols(void)
{
	struct kprobe kp;
	int ret;

	if (!real_fwnode_present) {
		memset(&kp, 0, sizeof(kp));
		kp.symbol_name = "fwnode_property_present";
		ret = register_kprobe(&kp);
		if (ret < 0) {
			pr_err("tcpm_drp_shim: fwnode_property_present not found (%d)\n", ret);
			return ret;
		}
		real_fwnode_present = (bool (*)(const struct fwnode_handle *,
						const char *))kp.addr;
		unregister_kprobe(&kp);
	}

	if (!real_kallsyms_lookup_name) {
		memset(&kp, 0, sizeof(kp));
		kp.symbol_name = "kallsyms_lookup_name";
		ret = register_kprobe(&kp);
		if (ret < 0) {
			pr_err("tcpm_drp_shim: kallsyms_lookup_name not found (%d)\n", ret);
			return ret;
		}
		real_kallsyms_lookup_name = (unsigned long (*)(const char *))kp.addr;
		unregister_kprobe(&kp);
	}

	if (!real_fwnode_present || !real_kallsyms_lookup_name)
		return -ENOENT;
	return 0;
}

struct probe_slot {
	struct typec_capability *cap;
	struct fwnode_handle *fwnode;
};

static bool armed;
static atomic_t patched;
static atomic_t live_runs;
static int last_live_rc;
static struct proc_dir_entry *proc_shim;

static int fwcap_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct probe_slot *s = (struct probe_slot *)ri->data;

	if (!READ_ONCE(armed))
		return 1;
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
	if (!cap || regs_return_value(regs) != 0)
		return 0;
	if (cap->type > TYPEC_PORT_DRP || cap->data > TYPEC_PORT_DRD)
		return 0;
	if (cap->type != TYPEC_PORT_SNK)
		return 0;
	fwnode = s->fwnode ? s->fwnode : cap->fwnode;
	if (!fwnode || !real_fwnode_present || !real_fwnode_present(fwnode, "source-pdos"))
		return 0;

	cap->type = TYPEC_PORT_DRP;
	cap->data = TYPEC_PORT_DRD;
	cap->prefer_role = TYPEC_SINK;
	pr_info("tcpm_drp_shim: probe-time port promoted to DRP (patch #%d)\n",
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

/*
 * Promote an already-registered /sys/class/typec/port0 in place without
 * unbinding the TCPC driver.
 *
 * In drivers/usb/typec/class.h:
 *   struct typec_port {
 *       unsigned int id;
 *       struct device dev;
 *       ...
 *       int prefer_role;
 *       enum typec_data_role data_role;
 *       enum typec_role pwr_role;
 *       enum typec_role vconn_role;
 *       enum typec_pwr_opmode pwr_opmode;
 *       enum typec_port_type port_type;
 *       struct mutex port_type_lock;
 *       struct mutex partner_link_lock;
 *       enum typec_orientation orientation;
 *       struct typec_switch *sw;
 *       struct typec_mux *mux;
 *       struct typec_retimer *retimer;
 *       const struct typec_capability *cap;
 *       const struct typec_operations *ops;
 *       ...
 *   };
 *
 * Because port->cap is a kmemdup() of struct typec_capability with
 * cap->fwnode == port->dev.fwnode and cap->ops == port->ops, scanning
 * the pointer slots after port->dev for a pair (cap_cand, ops_cand) where
 * cap_cand->fwnode == dev->fwnode && cap_cand->ops == ops_cand locates
 * port->cap and port->ops with zero hardcoded struct offsets.
 */
static int promote_live_port0(void)
{
	struct class *cls;
	struct device *dev;
	struct typec_port *port;
	struct fwnode_handle *fwnode;
	void **slots;
	struct typec_capability *cap = NULL;
	const struct typec_operations *ops = NULL;
	int i, ret = 0;

	atomic_inc(&live_runs);

	if (!real_kallsyms_lookup_name || !real_fwnode_present) {
		last_live_rc = -ENODEV;
		return -ENODEV;
	}
	cls = (struct class *)real_kallsyms_lookup_name("typec_class");
	if (!cls) {
		pr_err("tcpm_drp_shim: typec_class not found\n");
		last_live_rc = -ENOENT;
		return -ENOENT;
	}
	dev = class_find_device(cls, NULL, "port0", device_match_name);
	if (!dev) {
		pr_err("tcpm_drp_shim: port0 not found\n");
		last_live_rc = -ENODEV;
		return -ENODEV;
	}

	fwnode = dev->fwnode;
	if (!fwnode || !real_fwnode_present(fwnode, "source-pdos")) {
		pr_info("tcpm_drp_shim: port0 has no source-pdos, skipping\n");
		put_device(dev);
		last_live_rc = 0;
		return 0;
	}

	/* In struct typec_port, `unsigned int id` precedes `struct device dev`
	 * (8-byte aligned on arm64, so offsetof(struct typec_port, dev) == 8). */
	port = (struct typec_port *)((char *)dev - 8);
	slots = (void **)((char *)(dev + 1));

	for (i = 0; i < 48; i++) {
		struct typec_capability *c = (struct typec_capability *)slots[i];
		const struct typec_operations *o = (const struct typec_operations *)slots[i + 1];

		if ((unsigned long)c < PAGE_OFFSET || (unsigned long)o < PAGE_OFFSET)
			continue;
		if (c->fwnode == fwnode && c->ops == o &&
		    c->type <= TYPEC_PORT_DRP && c->data <= TYPEC_PORT_DRD) {
			cap = c;
			ops = o;
			break;
		}
	}

	if (!cap || !ops) {
		pr_err("tcpm_drp_shim: cap/ops pair not matched in port0\n");
		put_device(dev);
		last_live_rc = -EFAULT;
		return -EFAULT;
	}

	if (cap->type == TYPEC_PORT_DRP) {
		pr_info("tcpm_drp_shim: port0 already DRP\n");
		put_device(dev);
		last_live_rc = 0;
		return 0;
	}

	/* Promote capability block in place. */
	cap->type = TYPEC_PORT_DRP;
	cap->data = TYPEC_PORT_DRD;
	cap->prefer_role = TYPEC_SINK;

	/* Also update port->port_type in the 32-bit int slots right after
	 * struct ida mode_ids + struct usb_power_delivery *pd. Scan the first
	 * 16 u32 words of slots[] for the pattern:
	 *   [prefer_role (-1..1), data_role (0..1), pwr_role (0..1),
	 *    vconn_role (0..1), pwr_opmode (0..3), port_type == TYPEC_PORT_SNK (1)]
	 */
	{
		u32 *words = (u32 *)slots;
		int w;

		for (w = 0; w < 20; w++) {
			s32 pref = (s32)words[w];
			u32 dr = words[w + 1];
			u32 pr = words[w + 2];
			u32 vr = words[w + 3];
			u32 op = words[w + 4];
			u32 pt = words[w + 5];

			if (pref >= -1 && pref <= 1 &&
			    dr <= 1 && pr <= 1 && vr <= 1 && op <= 3 &&
			    pt == TYPEC_PORT_SNK) {
				words[w] = TYPEC_SINK;
				words[w + 5] = TYPEC_PORT_DRP;
				pr_info("tcpm_drp_shim: updated port->port_type at word +%d\n", w + 5);
				break;
			}
		}
	}

	/* Call tcpm_port_type_set(port, TYPEC_PORT_DRP) to flip tcpm->port_type
	 * and kick PORT_RESET -> DRP toggling. */
	if (ops->port_type_set) {
		ret = ops->port_type_set(port, TYPEC_PORT_DRP);
		pr_info("tcpm_drp_shim: ops->port_type_set(DRP) -> %d\n", ret);
	}
	if (ops->try_role) {
		int tr = ops->try_role(port, TYPEC_SOURCE);
		pr_info("tcpm_drp_shim: ops->try_role(SOURCE) -> %d\n", tr);
	}

	atomic_inc(&patched);
	last_live_rc = ret;
	put_device(dev);
	pr_info("tcpm_drp_shim: live port0 promoted to DRP (patch #%d)\n",
		atomic_read(&patched));
	return ret;
}

static ssize_t shim_write(struct file *file, const char __user *ubuf,
			  size_t count, loff_t *ppos)
{
	char buf[4];

	if (count == 0 || count > sizeof(buf) - 1)
		return -EINVAL;
	if (copy_from_user(buf, ubuf, count))
		return -EFAULT;
	buf[count] = '\0';

	if (buf[0] == '1' || buf[0] == '2') {
		WRITE_ONCE(armed, true);
		promote_live_port0();
	} else if (buf[0] == '0') {
		WRITE_ONCE(armed, false);
		pr_info("tcpm_drp_shim: disarmed\n");
	} else {
		return -EINVAL;
	}
	return count;
}

static int shim_show(struct seq_file *m, void *v)
{
	seq_printf(m, "armed=%d patched=%d reprobed=%d last=%d\n",
		   READ_ONCE(armed) ? 1 : 0, atomic_read(&patched),
		   atomic_read(&live_runs), last_live_rc);
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

	ret = resolve_symbols();
	if (ret)
		return ret;

	ret = register_kretprobe(&fwcap_probe);
	if (ret < 0) {
		pr_err("tcpm_drp_shim: kretprobe failed (%d)\n", ret);
		return ret;
	}

	proc_shim = proc_create("tcpm_drp_shim", 0660, NULL, &shim_ops);
	if (!proc_shim) {
		unregister_kretprobe(&fwcap_probe);
		return -ENOMEM;
	}

	WRITE_ONCE(armed, true);
	/* Immediately promote port0 if it is already registered. */
	promote_live_port0();
	return 0;
}

static void __exit tcpm_drp_shim_exit(void)
{
	WRITE_ONCE(armed, false);
	if (proc_shim)
		proc_remove(proc_shim);
	unregister_kretprobe(&fwcap_probe);
}

module_init(tcpm_drp_shim_init);
module_exit(tcpm_drp_shim_exit);
