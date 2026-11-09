import AsyncStorage from '@react-native-async-storage/async-storage';
import { StatusBar } from 'expo-status-bar';
import { ComponentProps, useEffect, useRef, useState } from 'react';
import {
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { router } from 'expo-router';
import { ChatState, Config, Line, fetchState, sendText } from '../bridge';
import { CHOICES, Choice, Palette, palette } from '../theme';
import { SIZE, TOUCH_MIN } from '../typography.ts';

const CONFIG_KEY = 'herdr-talk.config';
const PREFS_KEY = 'herdr-talk.prefs';
const POLL_MS = 1000;
const FONT = 'Galmuri11';

type Prefs = { theme: Choice; crt: boolean };
const DEFAULT_PREFS: Prefs = { theme: 'Green', crt: true };

export default function Index() {
  const [loaded, setLoaded] = useState(false);
  const [config, setConfig] = useState<Config | null>(null);
  const [prefs, setPrefs] = useState<Prefs>(DEFAULT_PREFS);

  useEffect(() => {
    (async () => {
      const [savedConfig, savedPrefs] = await Promise.all([
        AsyncStorage.getItem(CONFIG_KEY),
        AsyncStorage.getItem(PREFS_KEY),
      ]);
      if (savedConfig) setConfig(JSON.parse(savedConfig));
      if (savedPrefs) setPrefs({ ...DEFAULT_PREFS, ...JSON.parse(savedPrefs) });
      setLoaded(true);
    })();
  }, []);

  if (!loaded) return null;

  const colors = palette(prefs.theme);
  const updatePrefs = (next: Prefs) => {
    setPrefs(next);
    AsyncStorage.setItem(PREFS_KEY, JSON.stringify(next));
  };
  const connect = (next: Config) => {
    AsyncStorage.setItem(CONFIG_KEY, JSON.stringify(next));
    setConfig(next);
  };
  const disconnect = () => {
    AsyncStorage.removeItem(CONFIG_KEY);
    setConfig(null);
  };

  return (
    <SafeAreaView style={[styles.safe, { backgroundColor: colors.bezel }]}>
      <StatusBar style={prefs.theme === 'Light' ? 'dark' : 'light'} />
      {config ? (
        <ChatScreen
          config={config}
          colors={colors}
          prefs={prefs}
          onPrefs={updatePrefs}
          onDisconnect={disconnect}
        />
      ) : (
        <ConnectScreen colors={colors} onConnect={connect} />
      )}
    </SafeAreaView>
  );
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function ConnectScreen({ colors, onConnect }: { colors: Palette; onConnect: (config: Config) => void }) {
  const [url, setUrl] = useState('http://');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    const next = { url: url.trim(), token: token.trim() };
    setBusy(true);
    setError(null);
    try {
      await fetchState(next);
      onConnect(next);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <View style={[styles.connect, { backgroundColor: colors.background }]}>
      <Text style={[styles.title, { color: colors.accent }]}>herdr-talk</Text>
      <Text style={[styles.hint, { color: colors.muted }]}>
        PC 에서 bridge.js 를 실행하고, 창에 나온 주소와 토큰을 입력하세요.
      </Text>
      <Field label="주소" value={url} onChangeText={setUrl} colors={colors} placeholder="http://192.168.0.10:47860" />
      <Field label="토큰" value={token} onChangeText={setToken} colors={colors} />
      {error && <Text style={[styles.error, { color: colors.danger }]}>{error}</Text>}
      <Pressable
        onPress={submit}
        disabled={busy || !url.trim() || !token.trim()}
        style={[styles.button, { borderColor: colors.accent, opacity: busy ? 0.5 : 1 }]}
      >
        <Text style={[styles.buttonText, { color: colors.accent }]}>{busy ? '연결 중…' : '연결'}</Text>
      </Pressable>
      <Pressable
        onPress={() => router.push('/relay')}
        style={[styles.button, { borderColor: colors.accent, marginTop: 10 }]}
      >
        <Text style={[styles.buttonText, { color: colors.accent }]}>Supabase 연결</Text>
      </Pressable>
    </View>
  );
}

function Field({
  label,
  colors,
  ...props
}: { label: string; colors: Palette } & ComponentProps<typeof TextInput>) {
  return (
    <View style={styles.field}>
      <Text style={[styles.label, { color: colors.muted }]}>{label}</Text>
      <TextInput
        autoCapitalize="none"
        autoCorrect={false}
        placeholderTextColor={colors.muted}
        style={[styles.input, { color: colors.text, borderColor: colors.border, backgroundColor: colors.panel }]}
        {...props}
      />
    </View>
  );
}

function ChatScreen({
  config,
  colors,
  prefs,
  onPrefs,
  onDisconnect,
}: {
  config: Config;
  colors: Palette;
  prefs: Prefs;
  onPrefs: (next: Prefs) => void;
  onDisconnect: () => void;
}) {
  const [chat, setChat] = useState<ChatState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const listRef = useRef<FlatList<Line>>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const state = await fetchState(config);
        if (alive) {
          setChat(state);
          setError(null);
        }
      } catch (e) {
        if (alive) setError(errorText(e));
      }
    };
    tick();
    const timer = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [config]);

  const send = async () => {
    const text = draft.trim();
    if (!text) return;
    setDraft('');
    try {
      await sendText(config, text);
    } catch (e) {
      setError(errorText(e));
      setDraft(text);
    }
  };

  const body = (
    <View style={[styles.frame, { backgroundColor: colors.background, borderColor: colors.border }]}>
      <View style={[styles.topbar, { borderColor: colors.border }]}>
        <Text style={[styles.me, { color: colors.accent }]} numberOfLines={1}>
          {chat?.me?.name ?? '…'}
        </Text>
        {CHOICES.map((choice) => (
          <Chip
            key={choice}
            label={choice}
            active={prefs.theme === choice}
            colors={colors}
            onPress={() => onPrefs({ ...prefs, theme: choice })}
          />
        ))}
        <Chip label="CRT" active={prefs.crt} colors={colors} onPress={() => onPrefs({ ...prefs, crt: !prefs.crt })} />
        <Chip label="끊기" active={false} colors={colors} onPress={onDisconnect} />
      </View>

      <ScrollView horizontal style={[styles.peers, { borderColor: colors.border }]} contentContainerStyle={styles.peersInner}>
        {chat?.peers.length ? (
          chat.peers.map((peer) => (
            <Text key={`${peer.ip}:${peer.port}`} style={[styles.peer, { color: colors.muted, borderColor: colors.border }]}>
              {peer.name} ({peer.ip}:{peer.port})
            </Text>
          ))
        ) : (
          <Text style={[styles.peer, { color: colors.muted }]}>피어 없음</Text>
        )}
      </ScrollView>

      {error && <Text style={[styles.error, { color: colors.danger }]}>{error}</Text>}

      <FlatList
        ref={listRef}
        data={chat?.lines ?? []}
        keyExtractor={(_, index) => String(index)}
        style={styles.messages}
        onContentSizeChange={() => listRef.current?.scrollToEnd({ animated: false })}
        renderItem={({ item }) => (
          <Text style={[styles.line, { color: item.mine ? colors.mine : colors.text }]}>{item.text}</Text>
        )}
      />

      <View style={[styles.inputRow, { borderColor: colors.border }]}>
        <TextInput
          value={draft}
          onChangeText={setDraft}
          onSubmitEditing={send}
          placeholder="메시지 또는 /명령"
          placeholderTextColor={colors.muted}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="send"
          style={[styles.input, styles.inputText, { color: colors.text, borderColor: colors.border, backgroundColor: colors.panel }]}
        />
        <Pressable onPress={send} style={[styles.button, { borderColor: colors.accent }]}>
          <Text style={[styles.buttonText, { color: colors.accent }]}>보내기</Text>
        </Pressable>
      </View>
    </View>
  );

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : 'height'}>
      {prefs.crt ? (
        <View style={[styles.bezel, { backgroundColor: colors.bezel }]}>{body}</View>
      ) : (
        body
      )}
    </KeyboardAvoidingView>
  );
}

function Chip({
  label,
  active,
  colors,
  onPress,
}: {
  label: string;
  active: boolean;
  colors: Palette;
  onPress: () => void;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={[styles.chip, { borderColor: colors.border, backgroundColor: active ? colors.accent : 'transparent' }]}
    >
      <Text style={[styles.chipText, { color: active ? colors.background : colors.text }]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  safe: { flex: 1 },
  bezel: { flex: 1, padding: 10 },
  frame: { flex: 1, borderWidth: 1, borderRadius: 0 },
  connect: { flex: 1, padding: 24, gap: 12, justifyContent: 'center' },
  title: { fontFamily: FONT, fontSize: SIZE.title, marginBottom: 4 },
  hint: { fontFamily: FONT, fontSize: SIZE.label, marginBottom: 8 },
  field: { gap: 4 },
  label: { fontFamily: FONT, fontSize: SIZE.label },
  input: { minHeight: TOUCH_MIN, fontFamily: FONT, borderWidth: 1, paddingHorizontal: 10, paddingVertical: 8, fontSize: SIZE.body },
  inputText: { flex: 1 },
  button: { minHeight: TOUCH_MIN, borderWidth: 1, paddingHorizontal: 14, paddingVertical: 10, alignItems: 'center', justifyContent: 'center' },
  buttonText: { fontFamily: FONT, fontSize: SIZE.body },
  error: { fontFamily: FONT, fontSize: SIZE.label, paddingHorizontal: 10, paddingVertical: 4 },
  topbar: { flexDirection: 'row', alignItems: 'center', flexWrap: 'wrap', gap: 6, padding: 8, borderBottomWidth: 1 },
  me: { fontFamily: FONT, fontSize: SIZE.body, marginRight: 4, maxWidth: 120 },
  chip: { minHeight: TOUCH_MIN, borderWidth: 1, paddingHorizontal: 8, paddingVertical: 4 },
  chipText: { fontFamily: FONT, fontSize: SIZE.label },
  peers: { flexGrow: 0, borderBottomWidth: 1 },
  peersInner: { padding: 8, gap: 6 },
  peer: { fontFamily: FONT, fontSize: SIZE.label, borderWidth: 1, paddingHorizontal: 6, paddingVertical: 2 },
  messages: { flex: 1, paddingHorizontal: 10 },
  line: { fontFamily: FONT, fontSize: SIZE.body, lineHeight: 22 },
  inputRow: { flexDirection: 'row', gap: 6, padding: 8, borderTopWidth: 1 },
});
