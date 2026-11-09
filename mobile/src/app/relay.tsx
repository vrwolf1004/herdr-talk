// Relay chat on the phone (Supabase only). The phone talks to Supabase itself: no PC, no LAN, no bridge.
// Settings, the two keys, the peer's confirmation by code, and one chat with one peer.

import { useEffect, useRef, useState } from 'react';
import { Alert, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { StatusBar } from 'expo-status-bar';
import { bytesToHex, hexToBytes } from '../relay/bytes.ts';
import { receiveLine, sealText, type Keys } from '../relay/engine.ts';
import { PUT_TRIES, RelayLink, findRow, getRow, putRow, socketUrl, type PeerRow } from '../relay/client.ts';
import { NonceCache, channelName, fingerprint, isKeyHex } from '../relay/protocol.ts';
import { addTrusted, clearSettings, defaultSettings, loadOrCreateKeys, loadOrCreateShortId, loadSettings, loadTrusted, randomBytes, replaceShortId, saveSettings } from '../relay/store.ts';
import { FIELD_IDS, emptySettings, keyLabel, missingFields, type FieldId, type RelaySettings } from '../relay/settings.ts';
import { palette } from '../theme';

const LABELS: Record<FieldId, string> = {
  url: 'Project URL',
  key: 'Publishable key',
  name: 'Display name',
  peer: 'Peer ID (8 characters)',
};

type Message = { id: number; sender: string; text: string; mine: boolean };

export default function RelayScreen() {
  const colors = palette('Green');
  const [settings, setSettings] = useState<RelaySettings>(emptySettings());
  const [draft, setDraft] = useState<RelaySettings>(emptySettings());
  const [keys, setKeys] = useState<Keys | null>(null);
  const [signingSeed, setSigningSeed] = useState<Uint8Array | null>(null);
  const [signingPublic, setSigningPublic] = useState('');
  const [shortId, setShortId] = useState('');
  const [trusted, setTrusted] = useState<string[]>([]);
  const [peer, setPeer] = useState<PeerRow | null>(null);
  const [status, setStatus] = useState('starting');
  const [notice, setNotice] = useState('');
  const [editing, setEditing] = useState(false);
  const [usingDefaults, setUsingDefaults] = useState(false);
  const [input, setInput] = useState('');
  const [messages, setMessages] = useState<Message[]>([]);
  const nextId = useRef(1);
  const link = useRef<RelayLink | null>(null);
  const cache = useRef(new NonceCache());
  const keysRef = useRef<Keys | null>(null);
  const peerRef = useRef<PeerRow | null>(null);
  const shortIdRef = useRef('');

  useEffect(() => {
    (async () => {
      const [{ settings: loaded, fromDefaults }, made, trustedKeys, myShortId] = await Promise.all([loadSettings(), loadOrCreateKeys(), loadTrusted(), loadOrCreateShortId()]);
      shortIdRef.current = myShortId;
      setShortId(myShortId);
      setSettings(loaded);
      setDraft(loaded);
      setUsingDefaults(fromDefaults);
      setKeys(made.keys);
      keysRef.current = made.keys;
      setSigningSeed(made.signingSeed);
      setSigningPublic(made.signingPublicHex);
      setTrusted(trustedKeys);
      if (missingFields(loaded).length === 0) {
        await connect(loaded, made.keys, made.signingSeed, trustedKeys);
      } else {
        setStatus('set the settings');
        setEditing(true);
      }
    })();
    return () => link.current?.close();
    // Loaded once on mount.
  }, []);

  async function connect(current: RelaySettings, own: Keys, seed: Uint8Array, trustedKeys: string[]) {
    const found = await findRow(current, current.peer);
    if (!('row' in found)) {
      setStatus(found.reason);
      return;
    }
    const row = found.row;
    peerRef.current = row;
    setPeer(row);
    if (!trustedKeys.includes(row.noisePk)) {
      setStatus('peer not verified yet: compare the code and confirm');
      return;
    }
    // Publish this phone's row (signed) once per start. 'taken' means another key has this short ID: take a new one and retry.
    const publish = () => putRow(current, seed, bytesToHex(own.noisePublic), shortIdRef.current, Math.floor(Date.now() / 1000), bytesToHex(randomBytes(16)), undefined);
    let outcome = await publish();
    for (let attempt = 1; outcome === 'taken' && attempt < PUT_TRIES; attempt++) {
      shortIdRef.current = await replaceShortId();
      setShortId(shortIdRef.current);
      outcome = await publish();
    }
    // A failure is shown, and the chat goes on.
    if (outcome !== 'ok') setNotice('could not publish this phone\'s row');
    const channel = channelName(own.noisePublic, hexToBytes(row.noisePk));
    link.current?.close();
    const next = new RelayLink(socketUrl(current.url, current.key), `realtime:${channel}`, {
      onStatus: (text) => setStatus(text),
      onEnvelope: (line) => {
        const peerNow = peerRef.current;
        if (!keysRef.current || !peerNow) return;
        const result = receiveLine(keysRef.current, hexToBytes(peerNow.noisePk), peerNow.name, line, cache.current, Math.floor(Date.now() / 1000));
        if (result.kind === 'shown') {
          setMessages((list) => [...list, { id: nextId.current++, sender: result.line.sender, text: result.line.text, mine: false }]);
        }
      },
    });
    link.current = next;
    next.open();
  }

  async function confirmPeer() {
    if (!peer) return;
    const updated = await addTrusted(peer.noisePk);
    setTrusted(updated);
    if (keys && signingSeed) {
      await connect(settings, keys, signingSeed, updated);
    }
  }

  async function save() {
    const missing = missingFields(draft);
    if (missing.length > 0) {
      setNotice(`not saved. missing: ${missing.map((id) => LABELS[id]).join(', ')}`);
      return;
    }
    try {
      await saveSettings(draft);
    } catch {
      setNotice('could not save the settings on this device');
      return;
    }
    setSettings(draft);
    setEditing(false);
    setNotice('saved');
    if (keys && signingSeed) {
      await connect(draft, keys, signingSeed, trusted);
    }
  }

  // Remove every saved setting after one confirmation. The trusted peer keys stay.
  function askClear() {
    Alert.alert('설정 지우기', '저장된 Supabase 설정을 모두 지웁니다. 계속할까요?', [
      { text: '취소', style: 'cancel' },
      {
        text: '지우기',
        style: 'destructive',
        onPress: async () => {
          link.current?.close();
          link.current = null;
          await clearSettings();
          // The app defaults come back after a clear.
          setSettings(defaultSettings());
          setDraft(defaultSettings());
          setUsingDefaults(true);
          setPeer(null);
          peerRef.current = null;
          setMessages([]);
          setEditing(true);
          setNotice('settings cleared');
          setStatus('set the settings');
        },
      },
    ]);
  }

  async function send() {
    const text = input.trim();
    if (!text || !keys || !peer || !link.current) return;
    // Right before sending: the row must still carry the verified key, and the link must be joined.
    const again = await getRow(settings, peer.pk);
    if (!again || !trusted.includes(again.noisePk) || again.noisePk !== peer.noisePk) {
      setNotice('peer not verified, not sent');
      return;
    }
    const line = sealText(
      keys,
      hexToBytes(peer.noisePk),
      text,
      Math.floor(Date.now() / 1000),
      bytesToHex(randomBytes(16)),
      randomBytes(32),
    );
    if (!link.current.sendEnvelope(line)) {
      setNotice('not connected, not sent');
      return;
    }
    setMessages((list) => [...list, { id: nextId.current++, sender: settings.name, text, mine: true }]);
    setInput('');
  }

  const ownNoise = keys ? bytesToHex(keys.noisePublic) : '';
  const code = keys && peer && isKeyHex(peer.noisePk) ? fingerprint(keys.noisePublic, hexToBytes(peer.noisePk)) : '';
  const isTrusted = !!peer && trusted.includes(peer.noisePk);

  return (
    <SafeAreaView style={[styles.root, { backgroundColor: colors.background }]}>
      <StatusBar style="light" />
      <View style={[styles.statusRow, { borderColor: colors.border }]}>
        <Text style={[styles.statusText, { color: colors.accent }]}>mode: Supabase · {status}</Text>
        <Pressable onPress={() => setEditing((value) => !value)}>
          <Text style={[styles.link, { color: colors.accent }]}>{editing ? 'close' : 'settings'}</Text>
        </Pressable>
      </View>
      {notice ? <Text style={[styles.notice, { color: colors.muted }]}>{notice}</Text> : null}
      {usingDefaults ? <Text style={[styles.notice, { color: colors.muted }]}>기본값 사용 중 (URL·키·상대 키는 앱 기본값, 표시 이름만 직접 입력)</Text> : null}

      {editing ? (
        <ScrollView style={styles.panel}>
          {FIELD_IDS.map((id) => (
            <View key={id} style={styles.field}>
              <Text style={[styles.label, { color: colors.muted }]}>{LABELS[id]}</Text>
              <TextInput
                style={[styles.input, { color: colors.text, borderColor: colors.border }]}
                value={draft[id]}
                onChangeText={(value) => setDraft({ ...draft, [id]: value })}
                autoCapitalize="none"
                autoCorrect={false}
                secureTextEntry={id === 'key'}
                placeholder={id === 'key' ? keyLabel(draft.key) : undefined}
                placeholderTextColor={colors.muted}
              />
            </View>
          ))}
          <Text style={[styles.label, { color: colors.muted }]}>This phone ID (give it to the peer):</Text>
          <Text selectable style={[styles.short, { color: colors.accent }]}>{shortId}</Text>
          <Text style={[styles.label, { color: colors.muted }]}>This phone public key (Ed25519, 64 characters):</Text>
          <Text selectable style={[styles.mono, { color: colors.text }]}>{signingPublic}</Text>
          <Text style={[styles.label, { color: colors.muted }]}>This phone Noise key (the peer confirms it):</Text>
          <Text selectable style={[styles.mono, { color: colors.text }]}>{ownNoise}</Text>
          <View style={styles.row}>
            <Pressable onPress={save} style={[styles.button, { borderColor: colors.accent }]}>
              <Text style={{ color: colors.accent }}>save</Text>
            </Pressable>
            <Pressable onPress={askClear} style={[styles.button, { borderColor: colors.danger }]}>
              <Text style={{ color: colors.danger }}>clear settings</Text>
            </Pressable>
          </View>
        </ScrollView>
      ) : (
        <>
          {peer && !isTrusted ? (
            <View style={[styles.panel, { borderColor: colors.border }]}>
              <Text style={{ color: colors.text }}>peer {peer.name} is not confirmed yet.</Text>
              <Text selectable style={[styles.mono, { color: colors.accent }]}>code: {code}</Text>
              <Text style={{ color: colors.muted }}>Check that the same code is on the peer screen, then confirm.</Text>
              <Pressable onPress={confirmPeer} style={[styles.button, { borderColor: colors.accent }]}>
                <Text style={{ color: colors.accent }}>confirm: the codes match</Text>
              </Pressable>
            </View>
          ) : null}
          <ScrollView style={styles.messages} contentContainerStyle={{ paddingBottom: 8 }}>
            {messages.map((message) => (
              <Text key={message.id} style={{ color: message.mine ? colors.mine : colors.text }}>
                {message.sender}: {message.text}
              </Text>
            ))}
          </ScrollView>
          <View style={styles.inputRow}>
            <TextInput
              style={[styles.input, styles.inputFlex, { color: colors.text, borderColor: colors.border }]}
              value={input}
              onChangeText={setInput}
              onSubmitEditing={send}
              placeholder="message"
              placeholderTextColor={colors.muted}
            />
            <Pressable onPress={send} style={[styles.button, { borderColor: colors.accent }]}>
              <Text style={{ color: colors.accent }}>send</Text>
            </Pressable>
          </View>
        </>
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  statusRow: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', padding: 10, borderBottomWidth: 1 },
  statusText: { fontSize: 13 },
  link: { fontSize: 13, textDecorationLine: 'underline' },
  notice: { paddingHorizontal: 10, paddingVertical: 4, fontSize: 12 },
  panel: { flexGrow: 0, padding: 10 },
  field: { marginBottom: 10 },
  label: { fontSize: 12, marginBottom: 4 },
  input: { borderWidth: 1, borderRadius: 4, paddingHorizontal: 8, paddingVertical: 6, fontSize: 14 },
  inputFlex: { flex: 1 },
  mono: { fontSize: 12, marginBottom: 8 },
  short: { fontSize: 28, fontWeight: 'bold', marginBottom: 6 },
  button: { borderWidth: 1, borderRadius: 4, paddingHorizontal: 12, paddingVertical: 8, alignSelf: 'flex-start', marginTop: 8 },
  messages: { flex: 1, paddingHorizontal: 10 },
  inputRow: { flexDirection: 'row', gap: 8, padding: 10, alignItems: 'center' },
  row: { flexDirection: 'row', gap: 10, marginTop: 4 },
});
