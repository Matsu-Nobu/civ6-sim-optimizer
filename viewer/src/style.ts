// Colours and short labels of what the map draws.

const TERRAIN: Record<string, string> = {
  GRASS: "#5f8f3e",
  PLAINS: "#a39b4a",
  DESERT: "#d8c48a",
  TUNDRA: "#8c8f7a",
  SNOW: "#e6eaee",
  COAST: "#4f86b8",
  OCEAN: "#2c4f7c",
};

/** the fill of a terrain: hills darker, mountains grey */
export function terrainColour(terrain: string): string {
  const t = terrain.replace("TERRAIN_", "");
  if (t.endsWith("_MOUNTAIN")) return "#6e6862";
  const base = TERRAIN[t.replace("_HILLS", "")] ?? "#555";
  return t.endsWith("_HILLS") ? shade(base, -0.22) : base;
}

export function isHills(terrain: string): boolean {
  return terrain.endsWith("_HILLS");
}

/** the mark of a feature: its colour and a one-character label */
export const FEATURE: Record<string, [string, string]> = {
  FEATURE_FOREST: ["#244d1f", "森"],
  FEATURE_JUNGLE: ["#1f5c3f", "密"],
  FEATURE_MARSH: ["#4d6b55", "湿"],
  FEATURE_FLOODPLAINS: ["#9b8a4a", "氾"],
  FEATURE_FLOODPLAINS_GRASSLAND: ["#7a8f3e", "氾"],
  FEATURE_FLOODPLAINS_PLAINS: ["#9b8a4a", "氾"],
  FEATURE_OASIS: ["#3f9f8f", "オ"],
  FEATURE_REEF: ["#6fb0c0", "礁"],
  FEATURE_ICE: ["#dfe8f0", "氷"],
  FEATURE_GEOTHERMAL_FISSURE: ["#a05a3a", "地"],
  FEATURE_VOLCANIC_SOIL: ["#5a4a44", "火"],
};

export const DISTRICT: Record<string, [string, string]> = {
  DISTRICT_CAMPUS: ["#3d7bd9", "学"],
  DISTRICT_COMMERCIAL_HUB: ["#d9a83d", "商"],
  DISTRICT_INDUSTRIAL_ZONE: ["#c0632f", "工"],
  DISTRICT_HANSA: ["#c0632f", "工"],
  DISTRICT_ENTERTAINMENT_COMPLEX: ["#9a52c7", "娯"],
  DISTRICT_BATH: ["#4fa9c7", "浴"],
  DISTRICT_AQUEDUCT: ["#4fa9c7", "水"],
  DISTRICT_GOVERNMENT: ["#c74f6d", "政"],
  DISTRICT_DIPLOMATIC_QUARTER: ["#7a8fa8", "外"],
  DISTRICT_HOLY_SITE: ["#e8e0b0", "聖"],
  DISTRICT_THEATER: ["#c76fa8", "劇"],
  DISTRICT_ENCAMPMENT: ["#8a3a3a", "兵"],
  DISTRICT_HARBOR: ["#3d5fd9", "港"],
  DISTRICT_NEIGHBORHOOD: ["#8a9a6a", "住"],
  DISTRICT_SPACEPORT: ["#888", "宇"],
  DISTRICT_WONDER: ["#e0c060", "驚"],
};

export function districtStyle(type: string): [string, string] {
  return DISTRICT[type] ?? ["#999", "区"];
}

export const IMPROVEMENT: Record<string, string> = {
  IMPROVEMENT_FARM: "農",
  IMPROVEMENT_MINE: "鉱",
  IMPROVEMENT_QUARRY: "石",
  IMPROVEMENT_PASTURE: "牧",
  IMPROVEMENT_CAMP: "狩",
  IMPROVEMENT_PLANTATION: "園",
  IMPROVEMENT_FISHING_BOATS: "漁",
  IMPROVEMENT_LUMBER_MILL: "材",
  IMPROVEMENT_FORT: "砦",
  IMPROVEMENT_ROMAN_FORT: "砦",
};

/** the colour of a play (one per column) */
export const PLAY_COLOURS = ["#ff5a5a", "#4fc3ff", "#ffd24f", "#9cff6f"];

function shade(hex: string, by: number): string {
  const n = parseInt(hex.slice(1), 16);
  const ch = (v: number) => Math.max(0, Math.min(255, Math.round(v + (by < 0 ? v * by : (255 - v) * by))));
  const r = ch((n >> 16) & 255), g = ch((n >> 8) & 255), b = ch(n & 255);
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, "0")}`;
}
