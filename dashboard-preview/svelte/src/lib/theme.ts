import {Hct, SchemeTonalSpot, MaterialDynamicColors, argbFromHex, hexFromArgb} from '@material/material-color-utilities';
export function applyTheme(seed: string, dark: boolean) {
  const scheme = new SchemeTonalSpot(Hct.fromInt(argbFromHex(seed)), dark, 0);
  const colors = MaterialDynamicColors as unknown as Record<string, {getArgb?: (scheme: SchemeTonalSpot)=>number}>;
  for (const key of Object.getOwnPropertyNames(colors)) {
    const role = colors[key];
    if (role && typeof role.getArgb === 'function') {
      const name = key.replace(/[A-Z]/g, character => '-' + character.toLowerCase());
      document.documentElement.style.setProperty('--md-sys-color-' + name, hexFromArgb(role.getArgb(scheme)));
    }
  }
  document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
}
