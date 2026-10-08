/*
 * aoc_vote_shim.c — gvotable AOC-vote shim for Tensor G5/G6 recovery.
 *
 * On laguna (Tensor G5: frankel/blazer/mustang/rango) and malibu
 * (Tensor G6: grizzly/cubs/kodiak/yogi), USB host mode is decided by the
 * `google-role-sw` election `USB_DR_EL`: host requires TCPCI=host AND
 * AOC=host (verified by disassembly of google-role-sw.ko +
 * aoc_usb_driver.ko on mustang 6.6/6.12 and grizzly 6.12 — voters
 * `TCPCI_COMB`/`AOC`, log `[votes] %s... [result] %s [downstream]`).
 *
 * The AOC vote is cast by aoc_usb_driver's usb_host_aoc_ready_work once
 * the AoC `usb_control` service runs (started by userspace `aocd`).
 * In recovery the AoC firmware never comes up (`deferred probe pending`,
 * `wait for aoc output ctrl` forever), so the vote never lands and host
 * is impossible — the same absurdity the old `otg_host_shim` bypassed on
 * zuma/gs101 by calling dwc3_otg_host_ready() directly (no such symbol
 * exists on G5/G6, so that shim is dead there by design).
 *
 * This module bypasses the whole AoC stack by casting the vote itself:
 *   h = gvotable_election_get_handle("USB_DR_EL");
 *   gvotable_cast_vote(h, "AOC", 1, 1);
 * which is exactly what google-role-sw's own probe does when the
 * `disable-aoc-voter` DT boolean is present — and what malibu ships in
 * stock DTBO as `host-mode-aoc-optional` (46 overlays on grizzly,
 * 9 on yogi; 0 on every laguna DT/DTBO). Both symbols are plain
 * EXPORT_SYMBOL in gvotable.ko (no import namespace — none of the three
 * stock consumers declares one; __ksymtab verified live on mustang
 * 6.6.98) and stable across 6.6/6.12 (identical call sites).
 *
 * This module resolves them at runtime with kprobes (same "Kprobe
 * Edition" design as otg_host_shim): no link-time dependency, so it
 * builds against any kernel tree — even one without gvotable in-tree
 * (gvotable.ko is a vendor DLKM; modpost would otherwise fail with
 * "undefined!"). The symbols only need to be LOADED before our worker
 * gives up (~30s of retries covers first-stage order).
 *
 * Constants mirror the stock call sites, not guesses:
 * - assert (1,1): role-sw probe with `disable-aoc-voter` in DT.
 * - retract (0,1): aoc_usb_driver work when AoC is not ready.
 *
 * The vote is sticky once cast (field-proven: mustang keeps host across
 * `stop aocd` + replug), but a stock aoc_usb_driver loaded AFTER us would
 * cast (0,1) while the firmware is down — hence the two delayed
 * re-asserts covering load-order races. Recovery never insmods
 * aoc_usb_driver anymore, so normally the first cast wins outright.
 *
 * Compatible with:
 * - laguna 6.6 stable (mustang cp2a/bp4a) and 6.12 beta (same election)
 * - malibu 6.12 (belt-and-suspenders there: stock DT already opens
 *   the gate, this vote only re-asserts the same state)
 *
 * Proc interfaces (mirror otg_host_shim):
 * /proc/aoc_vote_shim  — write "1"=assert AOC/host vote, "0"=retract; read returns state
 * /proc/aoc_vote_ready — read-only, returns "1" when the vote is asserted
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/seq_file.h>
#include <linux/workqueue.h>
#include <linux/jiffies.h>
#include <linux/delay.h>
#include <linux/kprobes.h>
#include <linux/string.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("gvotable AOC vote shim for Tensor G5/G6 recovery (replaces aocd)");

/* gvotable.ko entry points (plain EXPORT_SYMBOL, no namespace).
 * Resolved at runtime via kprobe — see file header. Parameter names
 * are ours; the VALUES are stock: assert (1,1), retract (0,1). */
static void *(*real_get_handle)(const char *name) = NULL;
static int (*real_cast_vote)(void *election, const char *voter, int v1, int v2) = NULL;

static int resolve_gvotable_symbols(void)
{
    struct kprobe kp;
    int ret;

    if (real_get_handle && real_cast_vote)
        return 0;
    memset(&kp, 0, sizeof(kp));
    kp.symbol_name = "gvotable_election_get_handle";
    ret = register_kprobe(&kp);
    if (ret < 0) {
        pr_err("aoc_vote_shim: symbol 'gvotable_election_get_handle' not found (gvotable.ko loaded? err=%d)\n",
               ret);
        return ret;
    }
    real_get_handle = (void *(*)(const char *))kp.addr;
    unregister_kprobe(&kp);

    memset(&kp, 0, sizeof(kp));
    kp.symbol_name = "gvotable_cast_vote";
    ret = register_kprobe(&kp);
    if (ret < 0) {
        pr_err("aoc_vote_shim: symbol 'gvotable_cast_vote' not found (err=%d)\n", ret);
        real_get_handle = NULL;
        return ret;
    }
    real_cast_vote = (int (*)(void *, const char *, int, int))kp.addr;
    unregister_kprobe(&kp);

    if (!real_get_handle || !real_cast_vote) {
        pr_err("aoc_vote_shim: kprobe succeeded but address is NULL\n");
        real_get_handle = NULL;
        real_cast_vote = NULL;
        return -ENOENT;
    }
    pr_info("aoc_vote_shim: resolved gvotable symbols (get_handle=%p cast_vote=%p)\n",
            real_get_handle, real_cast_vote);
    return 0;
}

#define ELECTION_NAME "USB_DR_EL"
#define VOTER_NAME "AOC"

/* The election appears at google-role-sw probe, which may run after us
 * (first-stage order varies by tree): retry ~30s before giving up. */
#define HANDLE_RETRIES 30
#define HANDLE_RETRY_MS 1000
/* Re-asserts after a successful cast: cover a stock aoc_usb_driver that
 * loads late and casts (0,1) while the firmware is down. */
#define REASSERT_1_MS 10000
#define REASSERT_2_MS 30000

static void *vote_handle;
static bool vote_active;
static struct proc_dir_entry *proc_shim;
static struct proc_dir_entry *proc_ready;
static struct delayed_work vote_work;
static int vote_step; /* 0 = acquiring, 1 = first re-assert done, 2 = settled */

static int cast_aoc(int v1)
{
    int ret;

    if (!real_get_handle || !real_cast_vote)
        return -ENODEV;
    if (!vote_handle) {
        vote_handle = real_get_handle(ELECTION_NAME);
        if (!vote_handle)
            return -ENODEV;
        pr_info("aoc_vote_shim: election '%s' handle %p\n", ELECTION_NAME, vote_handle);
    }
    ret = real_cast_vote(vote_handle, VOTER_NAME, v1, 1);
    if (ret)
        pr_err("aoc_vote_shim: cast_vote(%d) failed: %d\n", v1, ret);
    return ret;
}

/* Worker: resolve symbols (retries while gvotable.ko loads), acquire the
 * election, cast, then re-assert twice. Runs in process context. */
static void vote_work_fn(struct work_struct *work)
{
    int i, ret;

    if (!vote_active)
        return;
    for (i = 0; i < HANDLE_RETRIES && vote_active; i++) {
        if (!resolve_gvotable_symbols())
            break;
        msleep(HANDLE_RETRY_MS);
    }
    if (!real_get_handle || !real_cast_vote) {
        pr_err("aoc_vote_shim: gvotable symbols never appeared, vote NOT asserted\n");
        vote_active = false;
        return;
    }
    if (!vote_handle) {
        vote_handle = real_get_handle(ELECTION_NAME);
        if (!vote_handle) {
            pr_err("aoc_vote_shim: election '%s' never appeared, vote NOT asserted\n",
                   ELECTION_NAME);
            vote_active = false;
            return;
        }
        pr_info("aoc_vote_shim: election '%s' handle %p\n", ELECTION_NAME, vote_handle);
    }
    ret = real_cast_vote(vote_handle, VOTER_NAME, 1, 1);
    if (ret) {
        pr_err("aoc_vote_shim: cast_vote failed: %d\n", ret);
        vote_active = false;
        return;
    }
    pr_info("aoc_vote_shim: AOC vote asserted (host-capable)\n");
    if (vote_step == 0) {
        vote_step = 1;
        schedule_delayed_work(&vote_work, msecs_to_jiffies(REASSERT_1_MS));
    } else if (vote_step == 1) {
        vote_step = 2;
        schedule_delayed_work(&vote_work, msecs_to_jiffies(REASSERT_2_MS - REASSERT_1_MS));
    }
    /* vote_step == 2: settled, no further work. Manual re-poke via
     * /proc/aoc_vote_shim remains available to recovery. */
}

static ssize_t shim_write(struct file *file, const char __user *ubuf,
    size_t count, loff_t *ppos)
{
    char buf[4];
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

    if (activate == vote_active)
        return count;

    if (activate) {
        vote_active = true;
        vote_step = 0;
        schedule_delayed_work(&vote_work, 0);
        pr_info("aoc_vote_shim: assert requested\n");
    } else {
        cancel_delayed_work_sync(&vote_work);
        if (vote_handle)
            cast_aoc(0);
        vote_active = false;
        pr_info("aoc_vote_shim: vote retracted\n");
    }
    return count;
}

static int shim_show(struct seq_file *m, void *v)
{
    seq_printf(m, "%d\n", vote_active ? 1 : 0);
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
    seq_printf(m, "%d\n", vote_active ? 1 : 0);
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

static int __init aoc_vote_shim_init(void)
{
    proc_shim = proc_create("aoc_vote_shim", 0660, NULL, &shim_ops);
    if (!proc_shim) {
        pr_err("aoc_vote_shim: failed to create /proc/aoc_vote_shim\n");
        return -ENOMEM;
    }

    proc_ready = proc_create("aoc_vote_ready", 0444, NULL, &ready_ops);
    if (!proc_ready) {
        pr_err("aoc_vote_shim: failed to create /proc/aoc_vote_ready\n");
        proc_remove(proc_shim);
        return -ENOMEM;
    }

    INIT_DELAYED_WORK(&vote_work, vote_work_fn);
    vote_active = true;
    vote_step = 0;
    schedule_delayed_work(&vote_work, 0);
    pr_info("aoc_vote_shim: loaded, acquiring election '%s'\n", ELECTION_NAME);
    return 0;
}

static void __exit aoc_vote_shim_exit(void)
{
    cancel_delayed_work_sync(&vote_work);
    if (vote_active && vote_handle)
        cast_aoc(0);
    vote_active = false;

    if (proc_ready)
        proc_remove(proc_ready);
    if (proc_shim)
        proc_remove(proc_shim);

    pr_info("aoc_vote_shim: unloaded\n");
}

module_init(aoc_vote_shim_init);
module_exit(aoc_vote_shim_exit);
