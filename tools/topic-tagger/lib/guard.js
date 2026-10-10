// The archive invariant: nothing under readings/ changes during a run.
import { execFileSync } from 'node:child_process';

const git = (root, args) => {
  try {
    return execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 64 << 20 });
  } catch {
    return null; // not a git repo: caller records "unchecked"
  }
};

export const readingsStatus = (root) => git(root, ['status', '--porcelain', '--', 'readings']);
export const fullStatus = (root) => git(root, ['status', '--porcelain']);
