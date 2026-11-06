import { useFonts } from 'expo-font';
import { Stack } from 'expo-router';

export default function RootLayout() {
  const [loaded] = useFonts({ Galmuri11: require('../../assets/fonts/Galmuri11.ttf') });
  if (!loaded) return null;
  return <Stack screenOptions={{ headerShown: false }} />;
}
