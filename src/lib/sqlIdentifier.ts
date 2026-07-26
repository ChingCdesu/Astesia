export function quoteClickHouseIdentifier(identifier: string): string {
  const escaped = identifier
    .replace(/\\/g, '\\\\')
    .replace(/`/g, '\\`');
  return `\`${escaped}\``;
}
