#!/usr/bin/env python3
"""Deterministic Codex wire peer for exercising Rally's orchestration in CI."""

import json
import os
import pathlib
import subprocess
import sys
import urllib.request

home = pathlib.Path(os.environ["CODEX_HOME"])
native = "test-thread-" + home.name
turn = ""
active = False


def send(value):
    print(json.dumps(value), flush=True)


def emit(method, params):
    send({"method": method, "params": params})


def delta(text):
    emit("item/agentMessage/delta", {"threadId": native, "turnId": turn, "delta": text})


def finish(text):
    global active
    delta(text)
    emit(
        "item/completed",
        {
            "item": {
                "type": "agentMessage",
                "id": turn,
                "phase": "final_answer",
                "text": text,
            }
        },
    )
    emit("turn/completed", {"turn": {"id": turn, "status": "completed"}})
    active = False


def probe(query):
    request = urllib.request.Request(
        os.environ["RALLY_URL"] + "/api/search",
        json.dumps({"query": query}).encode(),
        {
            "Content-Type": "application/json",
            "Authorization": "Bearer " + os.environ["RALLY_TOKEN"],
        },
    )
    with urllib.request.urlopen(request) as response:
        return sorted({item["mission"] for item in json.load(response)})


for line in sys.stdin:
    item = json.loads(line)
    method = item.get("method")
    params = item.get("params", {})
    req = item.get("id")
    if method == "initialize":
        send({"id": req, "result": {"userAgent": "Rally test fixture"}})
    elif method in ("thread/start", "thread/resume"):
        if method == "thread/resume":
            assert params["threadId"] == native
        send({"id": req, "result": {"thread": {"id": native, "turns": []}}})
    elif method == "account/login/start":
        assert params["type"] == "chatgptAuthTokens"
        send({"id": req, "result": {"type": "chatgptAuthTokens"}})
    elif method == "turn/start":
        text = params["input"][0]["text"]
        turn = "turn-" + os.environ["RALLY_RUN"]
        (home / "last-input.txt").write_text(text)
        (home / "last-effort.txt").write_text(params["effort"])
        send({"id": req, "result": {"turn": {"id": turn}}})
        if text == "We got interrupted, continue":
            finish("Continued in the same native session.")
        elif "ASYNC_CARD" in text.rsplit("### Addressed by", 1)[-1]:
            emit(
                "item/completed",
                {
                    "item": {
                        "type": "agentMessage",
                        "id": "ask-colour",
                        "text": "Pick a highlight.",
                        "questions": [
                            {"title": "Which colour?", "options": ["Amber", "Blue"]}
                        ],
                    }
                },
            )
            finish("I can continue once you choose.")
        elif "<send_user_message_question_reply>" in text:
            assert (
                '"questionItemId":"ask-colour"' in text and '"answer":"Amber"' in text
            )
            finish("Applied the queued answer.")
        elif "ROUTING_FIXTURE " in text.rsplit("### Addressed by", 1)[-1]:
            from git_routing import worker_probe

            data, _ = json.JSONDecoder().raw_decode(text.rsplit("ROUTING_FIXTURE ", 1)[-1])
            finish(worker_probe(data))
        elif "GIT_MISSION_PROBE" in text.rsplit("### Addressed by", 1)[-1]:
            subprocess.run(
                [
                    "git",
                    "-C",
                    "project",
                    "push",
                    "origin",
                    "HEAD:refs/heads/mission-probe",
                ],
                check=True,
            )
            finish("Mission Git access verified.")
        elif "CHECK_ENV" in text.rsplit("### Addressed by", 1)[-1]:
            finish(
                "scope="
                + ",".join(
                    os.environ.get(k, "")
                    for k in ["MISSION_SAMPLE", "ONLY_ROLE", "CALENDAR_TOKEN"]
                )
            )
        elif "HOLD" in text.rsplit("### Addressed by", 1)[-1]:
            active = True
            delta("Working. ")
            if "QUESTION" in text.rsplit("### Addressed by", 1)[-1]:
                emit("item/agentMessage/delta", {"delta": "Reviewing scope. "})
                send(
                    {
                        "id": 500,
                        "method": "item/tool/requestUserInput",
                        "params": {
                            "isBlocking": False,
                            "questions": [
                                {
                                    "id": "scope",
                                    "question": "Which scope?",
                                    "options": [{"label": "Small"}],
                                }
                            ],
                        },
                    }
                )
        else:
            finish("Mission work complete.")
    elif method == "turn/steer":
        send({"id": req, "result": {"turnId": turn}})
        text = params["input"][0]["text"]
        if "CHECK_ENV" in text:
            finish(
                "scope="
                + ",".join(
                    os.environ.get(k, "")
                    for k in ["MISSION_SAMPLE", "ONLY_ROLE", "CALENDAR_TOKEN"]
                )
            )
        if "REFRESH_AUTH" in text:
            send(
                {
                    "id": 800,
                    "method": "account/chatgptAuthTokens/refresh",
                    "params": {"reason": "unauthorized"},
                }
            )
        if "PROBE" in text:
            delta("Sources=" + ",".join(probe("privatebudgetmarker")) + "; ")
        if "FINISH" in text:
            finish("Completed after steering.")
    elif req == 500 and "result" in item:
        delta("Question answered. ")
    elif req == 800 and "result" in item:
        assert item["result"]["accessToken"] == "refreshed-test-access"
        finish("Refreshed worker credential.")
