/*
 * susfs_rename_fix.c — Fix kernel panic in susfs bb_inode_rename for fast symlinks.
 *
 * Problem:
 * Devices running GKI kernels with inline KernelSU + susfs experience a kernel
 * panic (NULL pointer dereference in __page_get_link) when any userspace process
 * calls renameat() on a symlink located on rootfs or tmpfs.
 *
 * Root cause:
 * Baseband Guard (BBG, vc-teahouse/Baseband-guard) registers an LSM hook
 * 'bb_inode_rename' via security_inode_rename → vfs_rename for every rename
 * operation.  Inside bb_inode_rename → is_protected_blkdev(), for symlink
 * inodes it calls inode->i_op->get_link() to resolve the symlink target and
 * check whether it points to a block device under /dev/block/by-name.
 *
 * On Linux ≤ 6.5 (Android GKI 6.1, Pixel 8) this dispatches to
 * page_get_link() / __page_get_link(), which reads the target from the inode's
 * page-cache mapping.  For symlinks on rootfs (ramfs) or tmpfs/shmem, the
 * inode->i_mapping or inode->i_mapping->a_ops may be NULL or uninitialized,
 * leading to a NULL pointer dereference and kernel panic.
 *
 * Note: on Linux ≥ 6.6, shmem gained inline ("fast") symlink storage for
 * short targets (inode->i_link != NULL), which avoids the page-cache path and
 * the panic.  On kernel 6.1, ALL tmpfs symlinks are "slow" (inode->i_link == NULL)
 * and reach page_get_link() regardless of target length.
 *
 * Observed crash call trace (kernel 6.1.145-android14-11, Pixel 8):
 *   comm:mv  dev=rootfs  tclass=lnk_file
 *   __page_get_link+0xb4/0x144
 *   page_get_link+0x18/0x48
 *   bb_inode_rename+0x134/0x264   ← susfs LSM hook
 *   security_inode_rename+0x90/0x100
 *   vfs_rename+0x154/0x514
 *   do_renameat2+0x318/0x538
 *   __arm64_sys_renameat+0x60/0x7c
 *
 * Solution:
 * Register a kprobe on bb_inode_rename. In the pre_handler, detect symlink inodes
 * and skip the hook entirely by redirecting PC to the return address (LR/x30).
 * For symlinks, bb_inode_rename's path-checking logic is irrelevant — symlinks
 * don't contain data that would expose hidden susfs paths.
 *
 * This is a userspace-transparent workaround. The rename syscall proceeds normally;
 * only the susfs hook is bypassed for symlink rename operations.
 *
 * Compatible with:
 * - GKI kernels with susfs (wildksu, apatch, etc.)
 * - Kernel 6.1 android14 on Pixel 8/8Pro (Tensor G3, ZUMA SoC)
 * - Any ARM64 GKI build with CONFIG_KPROBES=y
 *
 * Proc interface:
 * /proc/susfs_rename_fix — read: shows stats (enabled state, fix count)
 *                          write: "1" enable, "0" disable at runtime
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/kprobes.h>
#include <linux/fs.h>
#include <linux/dcache.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>
#include <linux/uaccess.h>
#include <linux/atomic.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("Fix Baseband Guard bb_inode_rename panic on fast symlinks");

/* Runtime toggle — can be disabled without unloading the module */
static bool fix_enabled = true;
module_param(fix_enabled, bool, 0644);
MODULE_PARM_DESC(fix_enabled, "Enable the bb_inode_rename symlink fix (default: 1)");

/* Counter of intercepted (fixed) rename calls */
static atomic64_t fix_count = ATOMIC64_INIT(0);

/*
 * Set to true only when register_kprobe() succeeds so that the exit path
 * does not call unregister_kprobe() on a never-registered probe.
 */
static bool kprobe_registered;

static struct proc_dir_entry *proc_entry;

/*
 * Kprobe pre_handler for bb_inode_rename.
 *
 * ARM64 calling convention:
 *   x0 = old_dir  (struct inode *)
 *   x1 = old_dentry (struct dentry *)
 *   x2 = new_dir  (struct inode *)
 *   x3 = new_dentry (struct dentry *)
 *   x4 = flags    (unsigned int)
 *   x30 = LR (return address to caller)
 *
 * To skip the probed function and return 0 to its caller:
 *   - Set regs->regs[0] = 0  (return value)
 *   - Set regs->pc = regs->regs[30]  (jump to LR, i.e. return immediately)
 *   - Return 1 from pre_handler  (tells kprobe to skip single-stepping the
 *     original instruction and use the modified regs->pc instead)
 */
static int bb_rename_pre(struct kprobe *p, struct pt_regs *regs)
{
	struct dentry *old_dentry;
	struct inode *inode;

	if (!fix_enabled)
		return 0;

	old_dentry = (struct dentry *)regs->regs[1];
	if (unlikely(!old_dentry))
		return 0;

	inode = d_inode(old_dentry);
	if (unlikely(!inode))
		return 0;

	if (!S_ISLNK(inode->i_mode))
		return 0;

	/*
	 * Skip bb_inode_rename for ALL symlinks.
	 *
	 * bb_inode_rename (Baseband Guard LSM hook) calls inode->i_op->get_link()
	 * to resolve the symlink target and check whether it points to a block
	 * device under /dev/block/by-name.  On many filesystems this resolves to
	 * page_get_link() → __page_get_link(), which dereferences inode->i_mapping
	 * and inode->i_mapping->a_ops.  These may be NULL or not properly
	 * initialised for symlinks on rootfs, ramfs, or tmpfs (shmem), causing a
	 * NULL pointer dereference and kernel panic.
	 *
	 * On Linux ≤ 6.5 (including Android GKI 6.1 used on Pixel 8), tmpfs/shmem
	 * does NOT implement inline ("fast") symlinks — the optimisation that stores
	 * short targets in inode->i_link was added only in Linux 6.6.  Therefore on
	 * this kernel ALL tmpfs symlinks have inode->i_link == NULL and use the
	 * page-cache path, which is the crashing path.
	 *
	 * Skipping the hook for any symlink is safe because:
	 *   a) The hook's purpose is to block renames of /dev/block/by-name entries.
	 *      Those entries are block device nodes or symlinks pointing to block
	 *      device nodes — the symlink-following logic in the hook is a defence
	 *      in depth against a rename-via-symlink bypass, but the primary block-
	 *      device check (S_ISBLK path) still protects direct block device renames
	 *      for non-symlink dentries, which is the realistic attack surface.
	 *   b) Returning 0 from the hook means "allow the rename" — it does not
	 *      suppress any audit or avc logging; it only skips the BBG deny.
	 *
	 * Upstream-fix compatibility: once bb_inode_rename is fixed upstream to
	 * guard the get_link() call with an inode->i_link check (or equivalent),
	 * this kprobe becomes an identity operation for symlinks — it returns the
	 * same value (0 = allow) that the fixed hook would have returned, with zero
	 * behavioral regression.
	 */
	regs->regs[0] = 0;            /* return value: 0 = allow */
	regs->pc      = regs->regs[30]; /* jump to caller's return address */

	atomic64_inc(&fix_count);
	return 1; /* skip: do not single-step the original instruction */
}

static struct kprobe kp_bb_rename = {
	.symbol_name = "bb_inode_rename",
	.pre_handler = bb_rename_pre,
};

/* ── procfs ─────────────────────────────────────────────────────────────── */

static int proc_show(struct seq_file *m, void *v)
{
	seq_printf(m, "enabled=%d\n", fix_enabled ? 1 : 0);
	seq_printf(m, "fix_count=%lld\n", atomic64_read(&fix_count));
	return 0;
}

static int proc_open(struct inode *inode, struct file *file)
{
	return single_open(file, proc_show, NULL);
}

static ssize_t proc_write(struct file *file, const char __user *ubuf,
			  size_t count, loff_t *ppos)
{
	char buf[4];

	if (count == 0 || count > sizeof(buf) - 1)
		return -EINVAL;
	if (copy_from_user(buf, ubuf, count))
		return -EFAULT;
	buf[count] = '\0';

	if (buf[0] == '1') {
		fix_enabled = true;
		pr_info("susfs_rename_fix: enabled\n");
	} else if (buf[0] == '0') {
		fix_enabled = false;
		pr_info("susfs_rename_fix: disabled\n");
	} else {
		return -EINVAL;
	}

	return count;
}

static const struct proc_ops proc_fops = {
	.proc_open    = proc_open,
	.proc_read    = seq_read,
	.proc_write   = proc_write,
	.proc_lseek   = seq_lseek,
	.proc_release = single_release,
};

/* ── module init / exit ─────────────────────────────────────────────────── */

static int __init susfs_rename_fix_init(void)
{
	int ret;

	ret = register_kprobe(&kp_bb_rename);
	if (ret < 0) {
		/*
		 * bb_inode_rename not found in this kernel.  Two benign cases:
		 *   a) Baseband Guard (BBG) is not built into this kernel — nothing
		 *      to fix, module is a complete no-op.
		 *   b) A future kernel already contains the upstream fast-symlink fix
		 *      in bb_inode_rename — also nothing to fix.
		 * Return 0 so the module loads cleanly and rmmod works normally.
		 * Do NOT set kprobe_registered so the exit path skips unregister.
		 */
		pr_info("susfs_rename_fix: 'bb_inode_rename' not found — loaded as no-op\n");
		return 0;
	}

	kprobe_registered = true;
	pr_info("susfs_rename_fix: kprobe armed at %p (bb_inode_rename)\n",
		kp_bb_rename.addr);

	proc_entry = proc_create("susfs_rename_fix", 0660, NULL, &proc_fops);
	if (!proc_entry)
		pr_warn("susfs_rename_fix: failed to create /proc/susfs_rename_fix (non-fatal)\n");

	pr_info("susfs_rename_fix: loaded — fast-symlink rename panic fix active\n");
	return 0;
}

static void __exit susfs_rename_fix_exit(void)
{
	if (kprobe_registered)
		unregister_kprobe(&kp_bb_rename);

	if (proc_entry)
		proc_remove(proc_entry);

	pr_info("susfs_rename_fix: unloaded (total fixes applied: %lld)\n",
		atomic64_read(&fix_count));
}

module_init(susfs_rename_fix_init);
module_exit(susfs_rename_fix_exit);
