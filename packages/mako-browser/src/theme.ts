// Retro cream/sepia palettes, matching the terminal game's truecolor theme
// (crates/mako-cli/src/ui/theme.rs): cream card faces, sepia frames, amber
// accents, muted green/red for wins and losses.

export interface MakoTheme {
  bg: string;
  panel: string;
  ink: string;
  dim: string;
  frame: string;
  accent: string;
  cardFace: string;
  cardRed: string;
  cardBlack: string;
  cardBack: string;
  cardBackInk: string;
  win: string;
  lose: string;
  warn: string;
  font: string;
}

const MONO = "ui-monospace, 'SFMono-Regular', 'Cascadia Mono', Menlo, Consolas, 'Liberation Mono', monospace";

/** Light: cream paper, sepia ink. */
export const sepiaTheme: MakoTheme = {
  bg: '#f2e6c9',
  panel: '#e9d9b4',
  ink: '#3b2b1c',
  dim: '#7a6450',
  frame: '#b08954',
  accent: '#9a5a14',
  cardFace: '#fbf4e2',
  cardRed: '#a52a2a',
  cardBlack: '#281e14',
  cardBack: '#784828',
  cardBackInk: '#d6aa6e',
  win: '#4f6f2a',
  lose: '#a2442f',
  warn: '#b65a1e',
  font: MONO,
};

/** Dark: the terminal look (cream text on deep brown). */
export const terminalTheme: MakoTheme = {
  bg: '#1f1710',
  panel: '#2b2017',
  ink: '#f2e6c9',
  dim: '#968269',
  frame: '#b08954',
  accent: '#e8aa50',
  cardFace: '#f2e6c9',
  cardRed: '#a52a2a',
  cardBlack: '#281e14',
  cardBack: '#784828',
  cardBackInk: '#d6aa6e',
  win: '#a0be6e',
  lose: '#d26e5a',
  warn: '#e6783c',
  font: MONO,
};

export type ThemeProp = 'sepia' | 'terminal' | Partial<MakoTheme>;

export function resolveTheme(t: ThemeProp | undefined): MakoTheme {
  if (t === undefined || t === 'sepia') return sepiaTheme;
  if (t === 'terminal') return terminalTheme;
  return { ...sepiaTheme, ...t };
}
