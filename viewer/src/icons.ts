// The game's icons tools/game_icons.py wrote to data/icons/ (ICON_<type>.png). A missing icon is drawn as the
// colour-and-letter mark of style.ts instead.

const images = new Map<string, HTMLImageElement>();
let available = new Set<string>();
let onLoad: () => void = () => {};

/** reads the list of the icons there are; `redraw` is called as each icon arrives */
export async function loadIcons(redraw: () => void) {
  onLoad = redraw;
  try {
    const r = await fetch("data/icons/icons.json");
    if (r.ok) available = new Set(await r.json());
  } catch {
    available = new Set();
  }
}

/** the loaded icon of a type (DISTRICT_CAMPUS, RESOURCE_IRON, UNIT_BUILDER, ...), or null */
export function icon(type: string): HTMLImageElement | null {
  const name = `ICON_${type}`;
  if (!available.has(name)) return null;
  let img = images.get(name);
  if (!img) {
    img = new Image();
    img.onload = () => onLoad();
    img.src = `data/icons/${name}.png`;
    images.set(name, img);
  }
  return img.complete && img.naturalWidth ? img : null;
}
