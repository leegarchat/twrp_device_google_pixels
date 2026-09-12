/*
 * otg_host_shim.c — DWC3 USB OTG host-ready shim for Tensor SoC recovery.
 *
 * On Pixel 8/9 (Tensor G3/G4), DWC3 Exynos driver needs two flags for host mode:
 * - host_ready (set by AOC aoc_usb_driver.probe → dwc3_otg_host_ready(true))
 * - host_on    (set by sysfs dwc3_exynos_otg_id write → dwc3_exynos_host_event)
 *
 * AOC is not available in recovery (probe fails: EPROBE_DEFER, no mailbox).
 * This module replaces AOC by calling dwc3_otg_host_ready() directly.
 *
 * Compatible with:
 * - GKI kernels (dwc3-exynos-usb.ko as loadable module)
 * - Monolithic kernels (dwc3 built-in, e.g. Sultan)
 *
 * Design (Kprobe Edition):
 * This version uses kprobes to dynamically resolve the unexported symbol
 * 'dwc3_otg_host_ready' at runtime, avoiding link-time modpost errors
 * and Bazel dependency hell.
 *
 * Proc interfaces:
 * /proc/otg_host_shim  — write "1"=activate, "0"=deactivate; read returns state
 * /proc/otg_host_ready — read-only, returns "1" if host_ready is active
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/seq_file.h>
#include <linux/kprobes.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("DWC3 OTG host-ready shim for recovery (replaces AOC) - Kprobe Edition");

static int (*real_dwc3_otg_host_ready)(bool ready) = NULL;

static bool host_ready_active;
static struct proc_dir_entry *proc_shim;
static struct proc_dir_entry *proc_ready;

static int resolve_dwc3_symbol(void)
{
    struct kprobe kp = {
        .symbol_name = "dwc3_otg_host_ready"
    };
    int ret;

    ret = register_kprobe(&kp);
    if (ret < 0) {
        pr_err("otg_host_shim: symbol 'dwc3_otg_host_ready' not found! Is the DWC3 driver loaded? (err=%d)\n", ret);
        return ret;
    }

    real_dwc3_otg_host_ready = (int (*)(bool)) kp.addr;
    unregister_kprobe(&kp);

    if (!real_dwc3_otg_host_ready) {
        pr_err("otg_host_shim: kprobe succeeded but address is NULL\n");
        return -ENOENT;
    }

    pr_info("otg_host_shim: successfully resolved 'dwc3_otg_host_ready' at %p\n", real_dwc3_otg_host_ready);
    return 0;
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

    if (!real_dwc3_otg_host_ready) {
        pr_err("otg_host_shim: function pointer is null!\n");
        return -EFAULT;
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
    if (resolve_dwc3_symbol() < 0) {
        return -ENODEV;
    }

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

    host_ready_active = false;
    pr_info("otg_host_shim: loaded successfully (inactive, waiting for activation)\n");
    return 0;
}

static void __exit otg_host_shim_exit(void)
{
    if (host_ready_active && real_dwc3_otg_host_ready) {
        real_dwc3_otg_host_ready(false);
        host_ready_active = false;
    }

    if (proc_ready)
        proc_remove(proc_ready);
    if (proc_shim)
        proc_remove(proc_shim);

    pr_info("otg_host_shim: unloaded\n");
}

module_init(otg_host_shim_init);
module_exit(otg_host_shim_exit);