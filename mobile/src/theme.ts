// gui/src/extend/theme.rs 의 palette() 와 같은 색.
export const CHOICES = ['Dark', 'Light', 'Amber', 'Green'] as const;
export type Choice = (typeof CHOICES)[number];

/** A saved theme name back to a choice; anything else (or nothing saved) is the default, Green. */
export function parseChoice(text: string | null): Choice {
  return CHOICES.find((choice) => choice === text) ?? 'Green';
}

export type Palette = {
  background: string;
  panel: string;
  border: string;
  text: string;
  muted: string;
  accent: string;
  mine: string;
  danger: string;
  bezel: string;
};

const PALETTES: Record<Choice, Palette> = {
  Dark: {
    background: '#1a1a2e', panel: '#16213e', border: '#2a3a5c', text: '#e0e0e0', muted: '#b0bec5',
    accent: '#4fc3f7', mine: '#81d4fa', danger: '#e57373', bezel: '#3a3a4a',
  },
  Light: {
    background: '#f5f7fa', panel: '#ffffff', border: '#cfd8dc', text: '#1a1a2e', muted: '#546e7a',
    accent: '#0277bd', mine: '#01579b', danger: '#c62828', bezel: '#d7d0c0',
  },
  Amber: {
    background: '#000000', panel: '#0d0900', border: '#7a5400', text: '#ffb000', muted: '#b37a00',
    accent: '#ffd24d', mine: '#ffd24d', danger: '#ff5533', bezel: '#2a2a2a',
  },
  Green: {
    background: '#050a08', panel: '#0a140f', border: '#417d55', text: '#b4dcbe', muted: '#526b59',
    accent: '#46ff82', mine: '#46ff82', danger: '#ff5555', bezel: '#2a2a2a',
  },
};

/** The noise setting as saved: "on" or "off". */
export function noiseText(on: boolean): string {
  return on ? "on" : "off";
}

/** A saved noise setting back to a boolean; anything else (or nothing saved) is off. */
export function parseNoise(text: string | null): boolean {
  return text === "on";
}

export function palette(choice: Choice): Palette {
  return PALETTES[choice];
}
