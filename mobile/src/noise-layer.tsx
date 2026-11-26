// The noise overlay: thin horizontal scanlines and a fixed set of faint specks over the whole screen, drawn with plain views.
// No animation, never takes touches, and mounted only while the setting is on.

import { useMemo } from 'react';
import { StyleSheet, useWindowDimensions, View } from 'react-native';
import { DOT_SIZE, LINE_HEIGHT, dotPositions, scanlineYs } from './noise-effect.ts';

const FAINT = 0.12;

export function NoiseLayer({ color }: { color: string }) {
  const { width, height } = useWindowDimensions();
  const lines = useMemo(() => scanlineYs(height), [height]);
  const dots = useMemo(() => dotPositions(width, height), [width, height]);

  return (
    <View style={StyleSheet.absoluteFill} pointerEvents="none">
      {lines.map((y) => (
        <View key={`line-${y}`} style={{ position: 'absolute', left: 0, right: 0, top: y, height: LINE_HEIGHT, backgroundColor: color, opacity: FAINT }} />
      ))}
      {dots.map((dot, index) => (
        <View key={`dot-${index}`} style={{ position: 'absolute', left: dot.x, top: dot.y, width: DOT_SIZE, height: DOT_SIZE, backgroundColor: color, opacity: FAINT }} />
      ))}
    </View>
  );
}
