/*
 * empty_checker_module.c — inclusion probe for recovery-pixel-boot.
 *
 * A deliberately empty kernel module: init creates /proc/empty_check_gki
 * and returns 0, exit removes it. No parameters, no dependencies, no
 * exported symbols, no hardware access.
 *
 * Purpose: direct evidence whether the running kernel really runs our
 * .ko files. The checker (ko_picker::module_inclusion_works, strict
 * finit_module, NO force flags) loads the build matching this exact
 * kernel — and the verdict is the /proc FILE, not the syscall return
 * code: Sultan's integrated-modules path fakes RC=0 for unknown modules
 * without ever running init, so only the file proves inclusion:
 * - /proc/empty_check_gki exists  -> modular tree (GKI-style): a module
 *   miss is real trouble -> HW GUI fallback stays armed;
 * - absent after insmod            -> (partially) monolithic tree: misses
 *   are tolerated, no HW GUI fallback.
 * A vermagic mismatch fails the same way as ENOSYS here — both mean
 * "not our tree", which is exactly the distinction the verdict needs.
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("LeeGarChat");
MODULE_DESCRIPTION("Empty inclusion probe for recovery module loading");

static struct proc_dir_entry *proc_check;

static int check_show(struct seq_file *m, void *v)
{
	seq_printf(m, "1\n");
	return 0;
}

static int check_open(struct inode *inode, struct file *file)
{
	return single_open(file, check_show, NULL);
}

static const struct proc_ops check_ops = {
	.proc_open    = check_open,
	.proc_read    = seq_read,
	.proc_lseek   = seq_lseek,
	.proc_release = single_release,
};

static int __init empty_checker_init(void)
{
	proc_check = proc_create("empty_check_gki", 0444, NULL, &check_ops);
	if (!proc_check) {
		pr_err("empty_checker: failed to create /proc/empty_check_gki\n");
		return -ENOMEM;
	}
	pr_info("empty_checker: inclusion probe loaded\n");
	return 0;
}

static void __exit empty_checker_exit(void)
{
	if (proc_check)
		proc_remove(proc_check);
	pr_info("empty_checker: inclusion probe unloaded\n");
}

module_init(empty_checker_init);
module_exit(empty_checker_exit);
