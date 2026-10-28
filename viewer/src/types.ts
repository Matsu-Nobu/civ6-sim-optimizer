// The data tools/viewer_data.py writes (viewer/public/data/<name>.json).

/** x, y, terrain, feature, resource, river edges (bit 0 = north-east, then clockwise) */
export type Plot = [number, number, string, string | null, string | null, number];

/** an owned plot now: x, y, feature, improvement, yields to a citizen working it (null on a district's plot) */
export type Tile = [number, number, string | null, string | null, number[] | null];

export interface District {
  type: string;
  done: boolean;
  x: number;
  y: number;
  wonder: string | null;
}

export interface City {
  name: string;
  xy: [number, number];
  pop: number;
  item: string | null;
  worked: [number, number][];
  /** food, production, gold, science, culture, faith */
  yields: number[];
  buildings: string[];
  districts: District[];
  tiles: Tile[];
}

export interface TurnState {
  turn: number;
  /** food, production, gold, science, culture, faith of the turn */
  output: number[];
  gold: number;
  techs: number;
  /** the techs researched */
  tech_list: string[];
  civics: number;
  research: string | null;
  civic: string | null;
  government: string | null;
  policies: string[];
  /** "GOVERNOR_X:city id:titles" */
  governors: string[];
  /** "UNIT_X:count" */
  units: string[];
  /** x, y, charges, id (the same builder keeps its id from turn to turn; -1: none) */
  builders: [number, number, number, number][];
  /** settlers on their way: the city they left, target x, target y, the turn they found the city, and the way:
   * [x, y, the turn they enter the plot] for each plot after the city */
  settlers: [string, number, number, number, [number, number, number][]][];
  strategic: Record<string, number>;
  cities: City[];
}

export interface Play {
  label: string;
  plan: string;
  turns: TurnState[];
  /** turn -> the decisions taken in it, in Japanese */
  decisions: Record<string, string[]>;
  score: number;
  total: number;
}

export interface GameData {
  name: string;
  game: string;
  options: string[];
  width: number;
  height: number;
  plots: Plot[];
  plays: Play[];
  names: Record<string, string>;
  /** resource -> "bonus", "luxury" or "strategic" */
  resource_classes: Record<string, string>;
  /** resource -> the tech that shows it on the map (no entry: always shown) */
  resource_techs: Record<string, string>;
}

export interface IndexEntry {
  name: string;
  plays: string[];
}
