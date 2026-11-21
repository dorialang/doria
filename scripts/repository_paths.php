<?php

declare(strict_types=1);

/**
 * Paths that repository-wide checks never read: version-control data, build
 * output, installed packages and `.claude/worktrees/`, whose transient
 * checkouts duplicate files already checked at the root. Every check that
 * walks the whole tree uses this one rule.
 */
function is_skipped_path(string $path): bool
{
    foreach (['.git/', 'target/', 'node_modules/', '.claude/worktrees/'] as $skip) {
        if (str_contains($path, $skip)) {
            return true;
        }
    }

    return false;
}
