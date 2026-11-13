// Supabase room settings on the phone. Room joining and room chat are implemented separately.

import { useEffect, useState } from 'react';
import { Alert, KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { StatusBar } from 'expo-status-bar';
import { clearDisplayName, clearSettings, defaultSettings, loadOrCreateDisplayName, loadSettings, saveSettings } from '../relay/store.ts';
import { FIELD_IDS, emptySettings, keyLabel, missingFields, normalizeRoomCode, type FieldId, type RelaySettings } from '../relay/settings.ts';
import { palette } from '../theme';
import { SIZE, TOUCH_MIN } from '../typography.ts';

const LABELS: Record<FieldId, string> = {
  url: 'Project URL',
  key: 'Publishable key',
  roomCode: 'Room ID (8 characters)',
};

export default function RelayScreen() {
  const colors = palette('Green');
  const [settings, setSettings] = useState<RelaySettings>(emptySettings());
  const [draft, setDraft] = useState<RelaySettings>(emptySettings());
  const [status, setStatus] = useState('starting');
  const [notice, setNotice] = useState('');
  const [editing, setEditing] = useState(false);
  const [usingDefaults, setUsingDefaults] = useState(false);

  useEffect(() => {
    (async () => {
      const { settings: loaded, fromDefaults } = await loadSettings();
      const current: RelaySettings = { ...loaded, name: await loadOrCreateDisplayName() };
      setSettings(current);
      setDraft(current);
      setUsingDefaults(fromDefaults);
      if (missingFields(current).length === 0) {
        prepareRoom(current);
      } else {
        setStatus('set the settings');
        setEditing(true);
      }
    })();
  }, []);

  function prepareRoom(current: RelaySettings) {
    // TODO: Call join-room with current.roomCode and open the room relay after the room protocol is implemented.
    setStatus(`room ${current.roomCode} is ready to join`);
  }

  async function save() {
    const next = { ...draft, roomCode: normalizeRoomCode(draft.roomCode) };
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
    setDraft(next);
    setSettings(next);
    setEditing(false);
    setNotice('saved');
    prepareRoom(next);
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
          setNotice('settings cleared');
          setStatus('set the settings');
        },
      },
    ]);
  }

  return (
    <SafeAreaView style={[styles.root, { backgroundColor: colors.background }]}>
      <StatusBar style="light" />
      <KeyboardAvoidingView style={styles.root} behavior={Platform.OS === 'ios' ? 'padding' : 'height'}>
        <View style={[styles.statusRow, { borderColor: colors.border }]}>
          <Text style={[styles.statusText, { color: colors.accent }]}>mode: Supabase · {status}</Text>
          <Pressable onPress={() => setEditing((value) => !value)} style={{ minHeight: TOUCH_MIN, justifyContent: 'center' }}>
            <Text style={[styles.link, { color: colors.accent }]}>{editing ? 'close' : 'settings'}</Text>
          </Pressable>
        </View>
        {notice ? <Text style={[styles.notice, { color: colors.muted }]}>{notice}</Text> : null}
        {usingDefaults ? <Text style={[styles.notice, { color: colors.muted }]}>기본값 사용 중 (URL·키·방 ID는 앱 기본값)</Text> : null}

        {editing ? (
          <ScrollView style={styles.panel} keyboardShouldPersistTaps="handled">
            {FIELD_IDS.map((id) => (
              <View key={id} style={styles.field}>
                <Text style={[styles.label, { color: colors.muted }]}>{LABELS[id]}</Text>
                <TextInput
                  style={[styles.input, { color: colors.text, borderColor: colors.border }]}
                  value={draft[id]}
                  onChangeText={(value) => setDraft({ ...draft, [id]: value })}
                  autoCapitalize={id === 'roomCode' ? 'characters' : 'none'}
                  autoCorrect={false}
                  secureTextEntry={id === 'key'}
                  placeholder={id === 'key' ? keyLabel(draft.key) : undefined}
                  placeholderTextColor={colors.muted}
                />
              </View>
            ))}
            <View style={styles.row}>
              <Pressable onPress={save} style={[styles.button, { borderColor: colors.accent }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.accent }}>save</Text>
              </Pressable>
              <Pressable onPress={askClear} style={[styles.button, { borderColor: colors.danger }]}>
                <Text style={{ fontSize: SIZE.body, color: colors.danger }}>clear settings</Text>
              </Pressable>
            </View>
          </ScrollView>
        ) : (
          <View style={styles.panel}>
            <Text style={[styles.label, { color: colors.muted }]}>Room chat will be available after room joining is implemented.</Text>
            <Text style={[styles.label, { color: colors.muted }]}>Selected room: {settings.roomCode}</Text>
          </View>
        )}
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  statusRow: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', padding: 10, borderBottomWidth: 1 },
  statusText: { fontSize: SIZE.label },
  link: { fontSize: SIZE.label, textDecorationLine: 'underline' },
  notice: { paddingHorizontal: 10, paddingVertical: 4, fontSize: SIZE.label },
  panel: { flexGrow: 0, flexShrink: 1, padding: 10 },
  field: { marginBottom: 10 },
  label: { fontSize: SIZE.label, marginBottom: 4 },
  input: { minHeight: TOUCH_MIN, borderWidth: 1, borderRadius: 4, paddingHorizontal: 8, paddingVertical: 6, fontSize: SIZE.body },
  button: { minHeight: TOUCH_MIN, borderWidth: 1, borderRadius: 4, paddingHorizontal: 12, paddingVertical: 8, alignSelf: 'flex-start', marginTop: 8 },
  row: { flexDirection: 'row', gap: 10, marginTop: 4 },
});
