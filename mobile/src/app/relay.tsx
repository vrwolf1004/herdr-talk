// Supabase room on the phone: settings, joining, the member list, and the room chat.

import { useEffect, useRef, useState } from 'react';
import { Alert, BackHandler, KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { StatusBar } from 'expo-status-bar';
import { clearDisplayName, clearSettings, defaultSettings, loadOrCreateDisplayName, loadOrCreateKeys, loadOrCreateShortId, loadSettings, randomBytes, replaceShortId, saveSettings } from '../relay/store.ts';
import { PUT_TRIES, deleteRow, putRow, redeemSetupCode } from '../relay/client.ts';
import { bytesToHex } from '../relay/bytes.ts';
import { NonceCache } from '../relay/protocol.ts';
import { checkRow, createRoom, displayName, fetchRows, loadMembers, postBody, postMessageBody, roomPayload, sealDeliveries } from '../relay/room.ts';
import { DEFAULT_URL, FIELD_IDS, emptySettings, isHttpsUrl, isTrustCode, missingFields, normalizeRoomCode, normalizeSetupCode, type FieldId, type RelaySettings } from '../relay/settings.ts';
import { palette } from '../theme';
import { SIZE, TOUCH_MIN } from '../typography.ts';

const LABELS: Record<FieldId, string> = {
  setupCode: 'Setup code',
  trust: 'Trust code (8 digits)',
  roomCode: 'Room ID (8 characters)',
};

/** The project URL for the setup-code call: the app default from .env, else the built-in one. */
const setupUrl = () => process.env.EXPO_PUBLIC_RELAY_URL?.trim() || DEFAULT_URL;

const POLL_MS = 3000;
const MEMBERS_MS = 10000;
const MAX_LINES = 200;
const nowSeconds = () => Math.floor(Date.now() / 1000);
const randomHex = (length: number) => bytesToHex(randomBytes(length));
const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const reason = (error: unknown) => (error instanceof Error ? error.message : 'error');

type Line = { id: string; who: string; text: string; mine: boolean };
type Session = { run: { stopped: boolean }; settings: RelaySettings; noisePrivate: Uint8Array; noisePublic: Uint8Array; seed: Uint8Array; shortId: string };

export default function RelayScreen() {
  const colors = palette('Green');
  const [settings, setSettings] = useState<RelaySettings>(emptySettings());
  const [draft, setDraft] = useState<RelaySettings>(emptySettings());
  // A problem with the room (shown in the status line); success texts are not kept.
  const [problem, setProblem] = useState('');
  const [notice, setNotice] = useState('');
  const [editing, setEditing] = useState(false);
  const [usingDefaults, setUsingDefaults] = useState(false);
  const [members, setMembers] = useState<Map<string, string>>(new Map());
  const [lines, setLines] = useState<Line[]>([]);
  const [text, setText] = useState('');
  const [inRoom, setInRoom] = useState(false);
  const [left, setLeft] = useState(false);
  const leaving = useRef<Promise<void> | null>(null);
  const session = useRef<Session | null>(null);
  const membersRef = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    (async () => {
      const { settings: loaded, fromDefaults } = await loadSettings();
      const current: RelaySettings = { ...loaded, name: await loadOrCreateDisplayName() };
      setSettings(current);
      setDraft({ ...current, setupCode: '' }); // the setup-code field starts empty; the saved code is in the status line
      setUsingDefaults(fromDefaults);
      if (missingFields(current).length === 0) {
        startRoom(current);
      } else {
        setNotice('set the settings first');
        setEditing(true);
      }
    })();
    return () => {
      if (session.current) session.current.run.stopped = true;
    };
  }, []);

  /** Publish this phone's row (a new short ID is tried when another key has it). The short ID, or null when it could not be published. */
  async function publishRow(current: RelaySettings, noisePublic: Uint8Array, signingSeed: Uint8Array): Promise<string | null> {
    const noiseHex = bytesToHex(noisePublic);
    let shortId = await loadOrCreateShortId();
    let outcome = await putRow(current, signingSeed, noiseHex, shortId, nowSeconds(), randomHex(16));
    for (let tries = 1; outcome === 'taken' && tries < PUT_TRIES; tries++) {
      shortId = await replaceShortId();
      outcome = await putRow(current, signingSeed, noiseHex, shortId, nowSeconds(), randomHex(16));
    }
    return outcome === 'ok' ? shortId : null;
  }

  /** Make a new room owned by this phone and put its code in the room ID field (saving joins it). */
  async function makeRoom() {
    const current: RelaySettings = { ...settings, trust: draft.trust.trim() };
    if (!isHttpsUrl(current.url) || current.key.trim() === '') {
      setNotice('create room: save a setup code first');
      return;
    }
    if (!isTrustCode(current.trust)) {
      setNotice('create room: the trust code must be 8 digits');
      return;
    }
    setNotice('create room: working...');
    try {
      const { keys, signingSeed } = await loadOrCreateKeys();
      if ((await publishRow(current, keys.noisePublic, signingSeed)) === null) {
        setNotice("create room failed: this phone's row could not be published");
        return;
      }
      const code = await createRoom(current, signingSeed, nowSeconds(), randomHex(16));
      setDraft({ ...draft, roomCode: code });
      setNotice(`room created: ${code} (press join)`);
    } catch (error) {
      setNotice(`create room failed: ${reason(error)}`);
    }
  }

  /** Publish this phone's row, join the room, then loop: members every 10 s, inbox every 3 s, until the screen closes or restarts. */
  async function startRoom(current: RelaySettings) {
    if (session.current) session.current.run.stopped = true;
    setInRoom(false);
    setLeft(false);
    setProblem('');
    const run = { stopped: false };
    setLines([]);
    setMembers(new Map());
    membersRef.current = new Map();
    try {
      const { keys, signingSeed } = await loadOrCreateKeys();
      const shortId = await publishRow(current, keys.noisePublic, signingSeed);
      if (shortId === null) {
        setProblem("this phone's row could not be published");
        return;
      }
      setInRoom(true);
      session.current = { run, settings: current, noisePrivate: keys.noisePrivate, noisePublic: keys.noisePublic, seed: signingSeed, shortId };
      const cache = new NonceCache();
      const started = nowSeconds();
      const names = new Map<string, string>();
      let since: string | null = null;
      let nextMembers = 0;
      while (!run.stopped) {
        if (Date.now() >= nextMembers) {
          nextMembers = Date.now() + MEMBERS_MS;
          try {
            membersRef.current = await loadMembers(current, signingSeed, nowSeconds(), randomHex(16));
            setMembers(membersRef.current);
            setProblem('');
          } catch (error) {
            setProblem(`room join failed: ${reason(error)}`);
            await sleep(MEMBERS_MS);
            continue;
          }
        }
        try {
          const rows = await fetchRows(current, signingSeed, since, nowSeconds(), randomHex(16));
          for (const row of rows) {
            since = row.created_at;
            const shown = checkRow(row, membersRef.current, keys.noisePrivate, () => true, cache, nowSeconds(), started);
            if (shown === null) continue;
            names.set(row.sender_pk, shown.name);
            const line: Line = { id: row.message_id, who: displayName(names, row.sender_pk, shown.name), text: shown.text, mine: false };
            setLines((prev) => [...prev, line].slice(-MAX_LINES));
          }
        } catch (error) {
          setProblem(`room: ${reason(error)}`);
        }
        await sleep(POLL_MS);
      }
    } catch (error) {
      setProblem(`room could not start: ${reason(error)}`);
    }
  }

  /** Leave the room: stop the loops and delete this phone's row on the relay. Saving the settings joins again. */
  async function leave() {
    const now = session.current;
    if (!now) return;
    now.run.stopped = true;
    session.current = null;
    setInRoom(false);
    setLines([]);
    setMembers(new Map());
    membersRef.current = new Map();
    setLeft(true); // the quit button shows at once
    setNotice('leaving the room...');
    const done = deleteRow(now.settings, now.seed, now.shortId, nowSeconds(), randomHex(16)).then((deleted) => {
      setNotice(deleted ? 'left the room' : 'left, but the relay did not confirm the delete');
    });
    leaving.current = done;
    await done;
  }

  /** Close the app (Android). The row delete of a leave in progress finishes first. */
  async function quit() {
    await leaving.current;
    if (Platform.OS === 'android') BackHandler.exitApp();
    else setNotice('quit: close the app from the app switcher (iOS cannot close itself)');
  }

  /** One Noise message per room member, in one post. */
  async function send() {
    const now = session.current;
    const message = text.trim();
    if (!now || message === '') return;
    const recipients = [...membersRef.current].map(([pk, noise]) => ({ pk, noise }));
    if (recipients.length === 0) {
      setNotice('no other member in the room yet');
      return;
    }
    try {
      const ts = nowSeconds();
      const nonce = randomHex(16);
      const deliveries = sealDeliveries(now.noisePrivate, recipients, roomPayload(now.settings.name, message, ts, nonce), () => randomBytes(32));
      const ok = await postBody(now.settings, postMessageBody(now.seed, now.settings.roomCode, randomHex(16), ts, nonce, deliveries));
      if (!ok) {
        setNotice('not sent: the server refused the message');
        return;
      }
    } catch (error) {
      setNotice(`not sent: ${reason(error)}`);
      return;
    }
    setLines((prev) => [...prev, { id: randomHex(8), who: now.settings.name, text: message, mine: true }].slice(-MAX_LINES));
    setText('');
    setNotice('');
  }

  /** Save Setup code: redeem the typed code now and store its URL, key and code (the rest of the settings stay). */
  async function saveSetupCode() {
    const code = normalizeSetupCode(draft.setupCode);
    if (code === null) {
      setNotice(`${LABELS.setupCode}: 8 letters or digits (spaces and dashes are ignored)`);
      return;
    }
    setNotice('setup code: checking...');
    try {
      const { url, key } = await redeemSetupCode(code, setupUrl());
      const next = { ...settings, url, key, setupCode: code };
      await saveSettings(next);
      setSettings(next);
      setDraft({ ...draft, setupCode: '' });
      setNotice('Setup code saved: the URL and key are stored (not shown).');
    } catch (error) {
      setNotice(reason(error));
    }
  }

  /** Join: the final save of the room ID and trust code. A typed setup code is not redeemed here (Save setup code does that). */
  async function save() {
    if (draft.setupCode.trim() !== '') {
      setNotice('Save setup code first: Join does not redeem a typed setup code');
      return;
    }
    const next: RelaySettings = { ...settings, roomCode: normalizeRoomCode(draft.roomCode), trust: draft.trust.trim() };
    const missing = missingFields(next);
    if (missing.length > 0) {
      setNotice(`not saved. missing: ${missing.map((id) => LABELS[id]).join(', ')}`);
      return;
    }
    try {
      await saveSettings(next);
    } catch {
      setNotice('could not save the settings on this device');
      return;
    }
    setDraft({ ...next, setupCode: '' });
    setSettings(next);
    setEditing(false);
    setNotice('saved');
    startRoom(next);
  }

  function askClear() {
    Alert.alert('설정 지우기', '저장된 Supabase 설정을 모두 지웁니다. 계속할까요?', [
      { text: '취소', style: 'cancel' },
      {
        text: '지우기',
        style: 'destructive',
        onPress: async () => {
          await clearSettings();
          await clearDisplayName();
          const name = await loadOrCreateDisplayName();
          const next = { ...defaultSettings(), name };
          setSettings(next);
          setDraft(next);
          setUsingDefaults(true);
          setEditing(true);
          setProblem('');
          setNotice('settings cleared');
        },
      },
    ]);
  }

  // The status line as Python shows it (English), with the room and setup code; only a problem is added after them.
  const statusLine = [
    `peers: ${members.size}`,
    'CLOUD SERVICE',
    settings.roomCode ? `room ID: ${settings.roomCode}` : null,
    settings.roomCode ? `setup code: ${settings.setupCode || '-'}` : null,
    problem || null,
  ]
    .filter(Boolean)
    .join(' · ');

  return (
    <SafeAreaView style={[styles.root, { backgroundColor: colors.background }]}>
      <StatusBar style="light" />
      <KeyboardAvoidingView style={styles.root} behavior={Platform.OS === 'ios' ? 'padding' : 'height'}>
        <View style={[styles.statusRow, { borderColor: colors.border }]}>
          <Text style={[styles.statusText, { color: problem ? colors.danger : colors.accent }]}>{statusLine}</Text>
          <View style={styles.actions}>
            {inRoom ? (
              <Pressable onPress={leave} style={[styles.topButton, { borderColor: colors.danger }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.danger }}>leave</Text>
              </Pressable>
            ) : left ? (
              <Pressable onPress={quit} style={[styles.topButton, { borderColor: colors.danger }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.danger }}>quit</Text>
              </Pressable>
            ) : null}
            <Pressable onPress={() => setEditing((value) => !value)} style={[styles.topButton, { borderColor: colors.accent }]}>
              <Text style={{ fontSize: SIZE.body, color: colors.accent }}>{editing ? 'close' : 'settings'}</Text>
            </Pressable>
          </View>
        </View>
        {notice ? <Text style={[styles.notice, { color: colors.muted }]}>{notice}</Text> : null}
        {usingDefaults ? <Text style={[styles.notice, { color: colors.muted }]}>기본값 사용 중 (URL·키·방 ID는 앱 기본값)</Text> : null}

        {editing ? (
          <ScrollView style={styles.panel} keyboardShouldPersistTaps="handled">
            {FIELD_IDS.map((id) => (
              <View key={id} style={styles.field}>
                <Text style={[styles.label, { color: colors.muted }]}>{LABELS[id]}</Text>
                <View style={styles.inputRow}>
                  <TextInput
                    style={[styles.input, styles.grow, { color: colors.text, borderColor: colors.border }]}
                    value={draft[id]}
                    onChangeText={(value) => setDraft({ ...draft, [id]: value })}
                    autoCapitalize={id === 'trust' ? 'none' : 'characters'}
                    autoCorrect={false}
                    keyboardType={id === 'trust' ? 'number-pad' : 'default'}
                    placeholderTextColor={colors.muted}
                  />
                  {id === 'roomCode' ? (
                    <Pressable onPress={makeRoom} accessibilityRole="button" accessibilityLabel="create room" style={[styles.inputButton, { borderColor: colors.accent }]}>
                      <Text style={{ fontSize: SIZE.body, color: colors.accent }}>+</Text>
                    </Pressable>
                  ) : null}
                  {id === 'setupCode' ? (
                    <Pressable onPress={saveSetupCode} accessibilityRole="button" accessibilityLabel="Save setup code" style={[styles.inputButton, { borderColor: colors.accent }]}>
                      <Text style={{ fontSize: SIZE.label, color: colors.accent }}>Save setup code</Text>
                    </Pressable>
                  ) : null}
                </View>
                {id === 'setupCode' ? (
                  <Text style={[styles.label, { color: colors.muted }]}>Type the setup code, then Save setup code: the URL and key are saved (not shown)</Text>
                ) : null}
              </View>
            ))}
            <View style={styles.row}>
              <Pressable onPress={save} style={[styles.button, { borderColor: colors.accent }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.accent }}>join</Text>
              </Pressable>
              <Pressable onPress={askClear} style={[styles.button, { borderColor: colors.danger }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.danger }}>clear</Text>
              </Pressable>
            </View>
          </ScrollView>
        ) : (
          <View style={styles.chat}>
            <ScrollView style={styles.log} keyboardShouldPersistTaps="handled">
              {[...members].map(([pk]) => (
                <Text key={pk} style={[styles.label, { color: colors.muted }]}>
                  member {pk.slice(0, 8)}
                </Text>
              ))}
              {lines.map((line) => (
                <Text key={line.id} style={[styles.line, { color: line.mine ? colors.accent : colors.text }]}>
                  {line.who} &gt; {line.text}
                </Text>
              ))}
            </ScrollView>
            <View style={[styles.row, styles.send]}>
              <TextInput
                style={[styles.input, styles.grow, { color: colors.text, borderColor: colors.border }]}
                value={text}
                onChangeText={setText}
                onSubmitEditing={send}
                returnKeyType="send"
                placeholder="message"
                placeholderTextColor={colors.muted}
              />
              <Pressable onPress={send} style={[styles.button, { borderColor: colors.accent }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.accent }}>send</Text>
              </Pressable>
            </View>
          </View>
        )}
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  statusRow: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', padding: 10, borderBottomWidth: 1 },
  actions: { flexDirection: 'row', gap: 8 },
  topButton: { minHeight: TOUCH_MIN, borderWidth: 1, borderRadius: 4, paddingHorizontal: 12, justifyContent: 'center' },
  statusText: { fontSize: SIZE.label },
  notice: { paddingHorizontal: 10, paddingVertical: 4, fontSize: SIZE.label },
  panel: { flexGrow: 0, flexShrink: 1, padding: 10 },
  field: { marginBottom: 10 },
  label: { fontSize: SIZE.label, marginBottom: 4 },
  input: { minHeight: TOUCH_MIN, borderWidth: 1, borderRadius: 4, paddingHorizontal: 8, paddingVertical: 6, fontSize: SIZE.body },
  button: { minHeight: TOUCH_MIN, borderWidth: 1, borderRadius: 4, paddingHorizontal: 12, paddingVertical: 8, alignSelf: 'flex-start', marginTop: 8 },
  row: { flexDirection: 'row', gap: 10, marginTop: 4 },
  inputRow: { flexDirection: 'row', gap: 8 },
  inputButton: { minHeight: TOUCH_MIN, borderWidth: 1, borderRadius: 4, paddingHorizontal: 12, justifyContent: 'center' },
  chat: { flex: 1 },
  log: { flex: 1, padding: 10 },
  grow: { flex: 1 },
  line: { fontSize: SIZE.body, marginBottom: 4 },
  send: { padding: 10, alignItems: 'center' },
});
