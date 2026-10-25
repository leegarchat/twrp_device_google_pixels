/*
 * otg_host_shim.c — DWC3 USB OTG host-ready shim for Tensor SoC recovery.
 *
 * Two builds from one source (picked by the target kernel, no Kconfig changes):
 *
 * - GKI module (CONFIG_KPROBES=y): resolves dwc3_otg_host_ready via kprobe
 *   at init. Power (VBUS) stays in userspace: recovery-pixel-boot writes the
 *   gvotable CHARGER_MODE debugfs voter, which exists on GKI.
 *
 * - Sultan built-in (no KPROBES, no module support, no DEBUG_FS): DIRECT
 *   extern calls, no symbol lookup at all.
 *   Why direct: on this tree every driver is linked into one vmlinux, but
 *   LTO/GC silently drops functions that have no in-tree caller
 *   (dwc3_otg_host_ready, gvotable_* vanished from kallsyms exactly for
 *   this reason — kprobe/kallsyms versions could never find them).
 *   A plain extern reference IS the fix: it is a real caller, so the
 *   linker keeps the functions. Power is bridged the same way:
 *   /proc/otg_vbus votes CHARGER_MODE=49 (GBMS_USB_OTG_ON, the exact value
 *   proven by GKI's DEBUGFS_FORCE otg_on=1) from kernel space, because the
 *   debugfs voter path does not exist without DEBUG_FS.
 *   Nobody but recovery touches these nodes; in normal Android they are
 *   inert (the shim ships only in the recovery ramdisk kernel).
 *
 * No role-switch backend: zuma device-tree has no "usb-role-switch"
 * property on dwc3, so dwc3 never registers one (always -ENODEV at runtime).
 *
 * Proc interfaces:
 * /proc/otg_host_shim  — write "1"=activate, "0"=deactivate; read returns state
 * /proc/otg_host_ready — read-only, returns "1" if host_ready is active
 * /proc/otg_vbus       — write "1"=vote VBUS on, "0"=release; read returns state
 *                         (built-in/Sultan path only; absent on GKI module builds)
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/seq_file.h>

#ifdef CONFIG_KPROBES
#include <linux/kprobes.h>
#else
#include <misc/gvotable.h>

/* Direct link against the Google glue + voter core. Both are linked into
 * the same vmlinux on Sultan (tristate =m coerced to built-in without
 * module support), so these resolve at link time — no lookup, no GC. */
extern int dwc3_otg_host_ready(bool ready);
#endif

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("DWC3 OTG host-ready shim for recovery (kprobe module / direct built-in + in-kernel VBUS vote)");

static int (*real_dwc3_otg_host_ready)(bool ready) = NULL;

static bool host_ready_active;
static struct proc_dir_entry *proc_shim;
static struct proc_dir_entry *proc_ready;

#ifndef CONFIG_KPROBES
/* ---- In-kernel VBUS vote (Sultan path, no debugfs) ---- */

/* CHARGER_MODE value proven on GKI: debugfs force_int 49 -> otg_on=1. */
#define CHARGER_MODE_OTG_VAL 49

static struct proc_dir_entry *proc_vbus;
static bool vbus_active;
static struct gvotable_election *charger_mode_handle;

static int vbus_vote(bool on)
{
	int ret;

	if (!charger_mode_handle) {
		charger_mode_handle = gvotable_election_get_handle("CHARGER_MODE");
		if (!charger_mode_handle) {
			pr_err("otg_host_shim: no CHARGER_MODE election (charger driver loaded?)\n");
			return -ENODEV;
		}
	}

	ret = gvotable_cast_vote(charger_mode_handle, "otg_shim",
				 (void *)(long)(on ? CHARGER_MODE_OTG_VAL : 0),
				 on);
	if (ret) {
		pr_err("otg_host_shim: CHARGER_MODE vote %s failed: %d\n",
		       on ? "on" : "off", ret);
		return ret;
	}

	vbus_active = on;
	pr_info("otg_host_shim: VBUS vote %s (CHARGER_MODE=%d)\n",
		on ? "ON" : "OFF", on ? CHARGER_MODE_OTG_VAL : 0);
	return 0;
}

static ssize_t vbus_write(struct file *file, const char __user *ubuf,
			  size_t count, loff_t *ppos)
{
	char buf[4];
	bool on;
	int ret;

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

	if (on == vbus_active)
		return count;

	ret = vbus_vote(on);
	if (ret)
		return ret;

	return count;
}

static int vbus_show(struct seq_file *m, void *v)
{
	seq_printf(m, "%d\n", vbus_active ? 1 : 0);
	return 0;
}

static int vbus_open(struct inode *inode, struct file *file)
{
	return single_open(file, vbus_show, NULL);
}

static const struct proc_ops vbus_ops = {
	.proc_open    = vbus_open,
	.proc_read    = seq_read,
	.proc_write   = vbus_write,
	.proc_lseek   = seq_lseek,
	.proc_release = single_release,
};
#endif /* !CONFIG_KPROBES */

static int resolve_dwc3_symbol(void)
{
#ifdef CONFIG_KPROBES
	struct kprobe kp = {
		.symbol_name = "dwc3_otg_host_ready"
	};
	int ret;

	ret = register_kprobe(&kp);
	if (ret < 0) {
		pr_err("otg_host_shim: kprobe register failed (err=%d), glue loaded?\n",
		       ret);
		return ret;
	}

	real_dwc3_otg_host_ready = (int (*)(bool))kp.addr;
	unregister_kprobe(&kp);

	if (!real_dwc3_otg_host_ready) {
		pr_err("otg_host_shim: kprobe succeeded but address is NULL\n");
		return -ENOENT;
	}

	pr_info("otg_host_shim: resolved 'dwc3_otg_host_ready' at %p\n",
		real_dwc3_otg_host_ready);
	return 0;
#else
	/* Built-in: direct reference (see above). Nothing to resolve —
	 * the link itself guarantees presence. */
	real_dwc3_otg_host_ready = dwc3_otg_host_ready;
	pr_info("otg_host_shim: using direct 'dwc3_otg_host_ready'\n");
	return 0;
#endif
}

static ssize_t shim_write(struct file *file, const char __user *ubuf,
   size_t count, loff_t *ppos)
{
	char buf[4];
	int ret;
	bool activate;

	if (count == 0 || count > sizeof(buf) - 1)
		return -EINVAL;

	if (copy_from_user(buf, ubuf, count))
		return -EFAULT;
	buf[count] = '\0';

	if (buf[0] == '1')
		activate = true;
	else if (buf[0] == '0')
		activate = false;
	else
		return -EINVAL;

	if (activate == host_ready_active)
		return count;

	/* Lazy resolve: glue may probe after us (built-in init order). */
	if (!real_dwc3_otg_host_ready) {
		ret = resolve_dwc3_symbol();
		if (ret)
			return ret;
	}

	ret = real_dwc3_otg_host_ready(activate);
	if (ret) {
		pr_err("otg_host_shim: dwc3_otg_host_ready(%d) failed: %d\n",
		       activate, ret);
		return ret;
	}

	host_ready_active = activate;
	pr_info("otg_host_shim: host_ready=%s\n", activate ? "true" : "false");

	return count;
}

static int shim_show(struct seq_file *m, void *v)
{
	seq_printf(m, "%d\n", host_ready_active ? 1 : 0);
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

static int ready_show(struct seq_file *m, void *v)
{
	seq_printf(m, "%d\n", host_ready_active ? 1 : 0);
	return 0;
}

static int ready_open(struct inode *inode, struct file *file)
{
	return single_open(file, ready_show, NULL);
}

static const struct proc_ops ready_ops = {
	.proc_open    = ready_open,
	.proc_read    = seq_read,
	.proc_lseek   = seq_lseek,
	.proc_release = single_release,
};

static int __init otg_host_shim_init(void)
{
	/* Never fail here: glue/charger may not be probed yet.
	 * First write resolves. */
	proc_shim = proc_create("otg_host_shim", 0660, NULL, &shim_ops);
	if (!proc_shim) {
		pr_err("otg_host_shim: failed to create /proc/otg_host_shim\n");
		return -ENOMEM;
	}

	proc_ready = proc_create("otg_host_ready", 0444, NULL, &ready_ops);
	if (!proc_ready) {
		pr_err("otg_host_shim: failed to create /proc/otg_host_ready\n");
		proc_remove(proc_shim);
		return -ENOMEM;
	}

#ifndef CONFIG_KPROBES
	proc_vbus = proc_create("otg_vbus", 0660, NULL, &vbus_ops);
	if (!proc_vbus) {
		pr_err("otg_host_shim: failed to create /proc/otg_vbus\n");
		proc_remove(proc_ready);
		proc_remove(proc_shim);
		return -ENOMEM;
	}
	vbus_active = false;
#endif

	host_ready_active = false;
	pr_info("otg_host_shim: loaded (inactive, waiting for activation)\n");
	return 0;
}

static void __exit otg_host_shim_exit(void)
{
	if (host_ready_active && real_dwc3_otg_host_ready) {
		real_dwc3_otg_host_ready(false);
		host_ready_active = false;
	}

#ifndef CONFIG_KPROBES
	if (vbus_active)
		vbus_vote(false);
	if (proc_vbus)
		proc_remove(proc_vbus);
#endif

	if (proc_ready)
		proc_remove(proc_ready);
	if (proc_shim)
		proc_remove(proc_shim);

	pr_info("otg_host_shim: unloaded\n");
}

module_init(otg_host_shim_init);
module_exit(otg_host_shim_exit);
