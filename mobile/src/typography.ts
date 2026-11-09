// Font sizes and touch sizes shared by the app screens. A screen takes its numbers from here; it does not write its own.

/** body 16 for text the user reads or types, label 14 for names and small notes, title for the app name, id for the short ID. */
export const SIZE = {
  body: 16,
  label: 14,
  title: 26,
  id: 32,
} as const;

/** The smallest touch target for buttons and input rows (points). */
export const TOUCH_MIN = 44;
