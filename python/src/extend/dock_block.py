"""The herdr-dock block of the chat: the pane label and the peer count, refreshed while the chat runs."""

import herdr_api
from core.net import STOP
from i18n import t


# ---------------------------------------------------------------- herdr-dock 블록
# 공용 herdr-dock workspace 의 한 칸에 talk 대화창을 두고, 접속자 수를 에이전트 목록에 보여 준다.
# 블록을 클릭하면 그 pane 이 포커스되고 대화창이 그대로 보인다. 다른 dock plugin 과 자리 규칙을 맞춘다.

DOCK_KEY = "herdr-talk"
DOCK_REPORT_SECONDS = 2
DOCK_TOKEN_TTL_MS = 15_000
AGENT_ID = "talk"
SOURCE = "plugin:herdr-talk"


def keep_dock_block(pane_id, chat):
    """블록에 접속자 수를 계속 알린다. 대화가 끝나 STOP 이 설정되면 멈춘다."""
    while not STOP.is_set():
        herdr_api.try_request("pane.report_metadata", {
            "pane_id": pane_id,
            "source": SOURCE,
            "display_agent": t("dock.agent_name", count=len(chat.fresh_peers())),
            "tokens": {},
            "ttl_ms": DOCK_TOKEN_TTL_MS,
        })
        STOP.wait(DOCK_REPORT_SECONDS)


