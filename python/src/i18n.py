"""화면과 안내 문구 다국어 처리 (herdr-timekit 의 i18n 과 같은 방식).

번역은 locales/<언어>.json 에 "키": "문장" 으로 둔다. 문장 안의 {name} 은 t() 의 인자로 채운다.

언어 결정 순서 (앞의 것이 있으면 그것을 쓴다)
  1. 환경변수 HERDR_TALK_LANG
  2. 플러그인 설정 HERDR_PLUGIN_CONFIG_DIR/config.json 의 "lang"
  3. OS 언어 (Windows: GetUserDefaultUILanguage, Linux/macOS: LC_ALL / LC_MESSAGES / LANG)
  4. en (OS 언어를 지원하지 않거나 알 수 없을 때)

키가 현재 언어에 없으면 en, 그래도 없으면 키 이름을 그대로 보여 준다.
"""

import json
import locale
import os
import sys
from pathlib import Path

SUPPORTED = ("en", "ko", "ru", "fr", "es", "de", "ja")
DEFAULT = "en"
LOCALES_DIR = Path(__file__).resolve().parent / "locales"


class _KeepMissing(dict):
    """문장에 있는 {name} 을 인자로 못 채우면 그대로 남긴다 (번역 실수로 죽지 않게)."""

    def __missing__(self, key):
        return "{" + key + "}"


def normalize(value):
    """'ko_KR.UTF-8', 'ko-KR', 'KO' → 'ko'. 지원하지 않으면 None."""
    if not value:
        return None
    code = str(value).strip().replace("-", "_").split(".")[0].split("_")[0].lower()
    return code if code in SUPPORTED else None


def _from_plugin_config():
    config_dir = os.environ.get("HERDR_PLUGIN_CONFIG_DIR")
    if not config_dir:
        return None
    path = Path(config_dir) / "config.json"
    try:
        return normalize(json.loads(path.read_text(encoding="utf-8-sig")).get("lang"))
    except (OSError, ValueError, AttributeError):
        return None


def _from_os():
    if sys.platform == "win32":
        try:
            import ctypes

            language_id = ctypes.windll.kernel32.GetUserDefaultUILanguage()
            return normalize(locale.windows_locale.get(language_id))
        except (AttributeError, OSError):
            return None
    for name in ("LC_ALL", "LC_MESSAGES", "LANG"):
        value = os.environ.get(name)
        if value:
            # 처음 설정된 변수가 우선한다. 지원하지 않는 언어면 거기서 멈추고 en 으로 간다.
            return normalize(value)
    return None


def detect():
    return (normalize(os.environ.get("HERDR_TALK_LANG"))
            or _from_plugin_config()
            or _from_os()
            or DEFAULT)


def _load(language):
    try:
        return json.loads((LOCALES_DIR / f"{language}.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


_fallback = _load(DEFAULT)
_language = DEFAULT
_messages = _fallback


def set_language(language):
    """언어를 바꾼다. 지원하지 않는 값이면 en."""
    global _language, _messages
    _language = normalize(language) or DEFAULT
    _messages = _fallback if _language == DEFAULT else _load(_language)


def language():
    return _language


def t(key, **values):
    text = _messages.get(key) or _fallback.get(key) or key
    return text.format_map(_KeepMissing(values)) if values else text


set_language(detect())
