"""The name rules of the Python edition (the same as Rust talk-core chat.rs and store.rs): the list of random names,
picking a free one, numbering a clash, and cleaning a sender or peer name. No screen, no locale text."""

import itertools
import locale
import random
import re


NAMES_BY_LANGUAGE = {
    "ko": ("korean", ["민준", "서준", "도윤", "예준", "시우", "하준", "지호", "주원", "서연", "서윤"]),
    "en": ("english", ["Liam", "Olivia", "Noah", "Emma", "Oliver", "Ava", "Elijah", "Sophia", "James", "Isabella"]),
    "zh": ("chinese", ["Haoran", "Yifan", "Ziyi", "Jiahui", "Xinyi", "Yu", "Mei", "Jie", "Lin", "Ting"]),
    "ja": ("japanese", ["Haruto", "Yui", "Sota", "Hina", "Ren", "Sakura", "Yuto", "Riku", "Aoi", "Mio"]),
    "vi": ("vietnamese", ["Anh", "Minh", "Huong", "Linh", "Tuan", "Hoa", "Duc", "Thao", "Phuc", "Lan"]),
    "fr": ("french", ["Louis", "Gabriel", "Jules", "Lea", "Chloe", "Manon", "Lucas", "Hugo", "Camille", "Adam"]),
    "de": ("german", ["Ben", "Mia", "Emil", "Hannah", "Paul", "Lina", "Leon", "Sofia", "Finn", "Greta"]),
    "es": ("spanish", ["Lucia", "Martin", "Mateo", "Valentina", "Pablo", "Daniela", "Diego", "Paula", "Sergio", "Carmen"]),
    "ru": ("russian", ["Mikhail", "Alexander", "Maria", "Artem", "Anna", "Ivan", "Daria", "Dmitry", "Polina", "Egor"]),
    "id": ("indonesian", ["Budi", "Siti", "Agus", "Dewi", "Rizky", "Putri", "Adi", "Nur", "Fajar", "Ayu"]),
    "pt": ("portuguese", ["Miguel", "Helena", "Arthur", "Alice", "Heitor", "Laura", "Bernardo", "Julia", "Davi", "Theo"]),
}


def pick_name(names, taken=()):
    """Random name that is not in taken. If every name is taken, fall back to the whole list."""
    free = [name for name in names if name not in taken]
    return random.choice(free or names)


def unique_name(base, taken):
    """base with the smallest number from 2 that is not in taken, e.g. 지호 -> 지호2."""
    for number in itertools.count(2):
        candidate = f"{base[:30]}{number}"
        if candidate not in taken:
            return candidate


def default_name(taken=()):
    """Random name from the list matching the PC's language (e.g. Korean for a Korean locale)."""
    language = (locale.getlocale()[0] or "").lower()
    for code, (language_name, names) in NAMES_BY_LANGUAGE.items():
        if language.startswith(code) or language.startswith(language_name):
            return pick_name(names, taken)
    return pick_name([name for _, names in NAMES_BY_LANGUAGE.values() for name in names], taken)


def clean_text(value, limit):
    """Replace control characters with spaces and cut to limit characters."""
    return "".join(char if char.isprintable() else " " for char in value)[:limit]


def peer_name(value):
    """입장 알림으로 받은 피어 이름. 메시지 보낸 사람 이름과 같은 규칙 (제어 문자 공백, 32자) 으로 정리하고,
    정리 후 비면 unknown 을 쓴다. 이름 길이 규칙 자체는 여기서 정하지 않는다."""
    if not isinstance(value, str):
        return "unknown"
    name = clean_text(value, 32)
    return name if name.strip() else "unknown"


def squeeze_spaces(value):
    """Collapse each run of spaces to one space."""
    return re.sub(" +", " ", value)


def sender_name(value):
    """A sender name as shown: cleaned, spaces squeezed, trimmed, and "unknown" when nothing is left."""
    return squeeze_spaces(clean_text(value, 32)).strip() or "unknown"


_SEPARATORS = str.maketrans("", "", ":：>＞›")  # : ： > ＞ ›


def plain_sender(value):
    """The sender of a plain message. Colons and the ">" separators are removed, so the name cannot pass for the start of
    the text or imitate the "name > text" line form."""
    return sender_name(clean_text(value, 32).translate(_SEPARATORS))
