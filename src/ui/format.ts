// Size formatting. Uses **binary** units (1024-based) so the numbers match what
// Windows Explorer shows for the same folder - Explorer divides by 1024² / 1024³ but
// still labels it "MB"/"GB", so a decimal (÷1e6 / ÷1e9) formatter reads ~7% higher
// than the OS for the identical byte count (e.g. 914,000,000 B → Explorer "871 MB",
// not "914 MB"; 4,831,838,208 B → "4.50 GB", not "4.83 GB").
const KIB = 1024;
const MIB = 1024 * 1024;
const GIB = 1024 * 1024 * 1024;

/** Bytes → human size matching Explorer (GB/MB binary). `-` for missing/zero. */
export function formatBytes(bytes: number | null | undefined): string {
  if (!bytes || bytes <= 0) return "-";
  if (bytes >= GIB) return `${(bytes / GIB).toFixed(2)} GB`;
  if (bytes >= MIB) return `${(bytes / MIB).toFixed(0)} MB`;
  return `${(bytes / KIB).toFixed(0)} KB`;
}

/** Bytes → GiB as a number, for chart axes plotted in "GB". */
export const bytesToGiB = (bytes: number): number => bytes / GIB;
