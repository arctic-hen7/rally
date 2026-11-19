"""Offline Responses endpoint for exercising a real Codex executable inside Podman."""

import http.server
import json
import secrets
import time


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(
            json.dumps(
                {"object": "list", "data": [{"id": "gpt-6-luna", "object": "model"}]}
            ).encode()
        )

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        inputs = body.get("input", [])
        followup = "RALLY_FOLLOWUP_PROBE" in json.dumps(inputs)
        probe = followup or "RALLY_WORKSPACE_PROBE" in json.dumps(inputs)
        call_id = "call_rally_followup" if followup else "call_rally_probe"
        outputs = [
            entry
            for entry in inputs
            if entry.get("type") == "function_call_output"
            and entry.get("call_id") == call_id
        ]
        reply = "Rally container runtime verified."
        rid = "resp_" + secrets.token_hex(8)
        item = {
            "id": "msg_" + secrets.token_hex(8),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "phase": "final_answer",
            "content": [{"type": "output_text", "text": reply, "annotations": []}],
        }
        if probe and not outputs:
            args = {
                "cmd": (
                    'test "$MISSION_SMOKE_TOKEN" = '
                    + ("second" if followup else "first")
                    + ' && test "$AGENT_SMOKE_TOKEN" = agent'
                    + " && printf 'verified-%s' \"$MISSION_SMOKE_TOKEN\" > rally-probe.txt && cat rally-probe.txt"
                )
            }
            if "RALLY_GIT_PROBE" in json.dumps(inputs):
                args["cmd"] = (
                    "git -C project fetch origin && git -C project push origin HEAD:refs/heads/hosted-smoke && "
                    + args["cmd"]
                )
            item = {
                "id": "fc_" + call_id,
                "call_id": call_id,
                "type": "function_call",
                "name": "exec_command",
                "arguments": json.dumps(args),
                "status": "completed",
            }
        elif outputs:
            ok = "verified" in str(outputs[-1].get("output", "")) and "code 0" in str(
                outputs[-1].get("output", "")
            )
            reply = (
                "Rally container runtime verified."
                if ok
                else "Workspace command failed: "
                + str(outputs[-1].get("output", ""))[:600]
            )
            item["content"][0]["text"] = reply
        response = {
            "id": rid,
            "object": "response",
            "created_at": int(time.time()),
            "status": "completed",
            "model": "gpt-6-luna",
            "output": [item],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
        }
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        events = [
            {
                "type": "response.created",
                "response": dict(response, status="in_progress", output=[]),
            },
            {
                "type": "response.output_item.added",
                "output_index": 0,
                "item": dict(item, status="in_progress", content=[]),
            },
            {
                "type": "response.output_text.delta",
                "output_index": 0,
                "item_id": item["id"],
                "content_index": 0,
                "delta": reply,
            },
            {"type": "response.output_item.done", "output_index": 0, "item": item},
            {"type": "response.completed", "response": response},
        ]
        if item["type"] == "function_call":
            events[1]["item"] = dict(item, status="in_progress", arguments="")
            events[2] = {
                "type": "response.function_call_arguments.delta",
                "item_id": item["id"],
                "output_index": 0,
                "delta": item["arguments"],
            }
        for i, e in enumerate(events):
            e["sequence_number"] = i
            self.wfile.write(
                ("event: " + e["type"] + "\ndata: " + json.dumps(e) + "\n\n").encode()
            )
        self.wfile.flush()


if __name__ == "__main__":
    http.server.ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
