// gui/src/main.rs 의 palette() 와 같은 색.
export const CHOICES = ['Dark', 'Light', 'Amber', 'Green'] as const;
export type Choice = (typeof CHOICES)[number];

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
    background: '#000000', panel: '#00100a', border: '#1a7a36', text: '#33ff66', muted: '#1fa040',
    accent: '#99ffb3', mine: '#99ffb3', danger: '#ff5555', bezel: '#2a2a2a',
  },
};

export function palette(choice: Choice): Palette {
  return PALETTES[choice];
}
