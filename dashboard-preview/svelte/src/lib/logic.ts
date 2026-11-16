export function clock(seconds = 0) {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.floor(seconds)) : 0;
  const units = [Math.floor(total / 60) % 60, total % 60];
  if (total >= 3600) return [Math.floor(total / 3600), ...units.map(n => String(n).padStart(2, '0'))].join(':');
  return units[0] + ':' + String(units[1]).padStart(2, '0');
}

export function visibleRange(count: number, columns: number, row: number, scroll: number, height: number) {
  const firstRow = Math.max(0, Math.floor(scroll / row) - 1);
  const lastRow = Math.ceil((scroll + height) / row) + 1;
  return {start: Math.min(count, firstRow * columns), end: Math.min(count, lastRow * columns), offset: firstRow * row};
}

export function rememberPosition(storage: Pick<Storage, 'getItem' | 'setItem'>, url: string, position: number) {
  if (!url || !Number.isFinite(position) || position < 0) return;
  try {
    const previous = JSON.parse(storage.getItem('mb-progress') || '{}') as Record<string, number>;
    const entries = Object.entries(previous).filter(([key]) => key !== url).slice(-99);
    storage.setItem('mb-progress', JSON.stringify(Object.fromEntries([...entries, [url, position]])));
  } catch {}
}
export function savedPosition(storage: Pick<Storage, 'getItem'>, url: string) {
  try {
    const value: unknown = JSON.parse(storage.getItem('mb-progress') || '{}')[url];
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0;
  } catch { return 0; }
}

export function shortcut(key: string, editing: boolean, modified: boolean) {
  if (editing || modified) return null;
  return ({' ': 'toggle', ArrowLeft: 'back', ArrowRight: 'forward'} as Record<string, string>)[key] || null;
}
