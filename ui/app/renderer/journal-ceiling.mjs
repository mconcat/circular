const MIB = 1024n * 1024n;
const whole = value => BigInt(value != null && typeof value === 'object' && 'value' in value ? value.value : value);
const mib = bytes => `${(Number((bytes * 10n + MIB / 2n) / MIB) / 10).toFixed(1)} MiB`;

export function journalCeiling(page) {
  const journal = page?.anchor?.journal;
  const code = journal?.ceiling?.code;
  if (typeof code !== 'string') return null;
  const parts = [];
  if (journal.usage) {
    const u = Object.fromEntries(Object.entries(journal.usage).map(([name, value]) => [name, whole(value)]));
    if (u.bytes > u.arrivals_max_bytes) parts.push(`arrival journal ${mib(u.bytes)} over arrivals_max_mib ${mib(u.arrivals_max_bytes)}`);
    if (u.records > u.arrivals_max_records) parts.push(`${u.records} records over arrivals_max_records ${u.arrivals_max_records}`);
    if (u.file_bytes > u.total_max_bytes) parts.push(`journal files ${mib(u.file_bytes)} over total_max_mib ${mib(u.total_max_bytes)}`);
    if (!parts.length) parts.push(`arrival journal ${mib(u.bytes)}, ${u.records} records, journal files ${mib(u.file_bytes)}`);
  }
  return { code, text: `Journal over its ceiling${parts.length ? `: ${parts.join('; ')}` : ''}` };
}
