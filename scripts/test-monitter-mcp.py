#!/usr/bin/env python3
import http.server, json, os, subprocess, sys, threading, unittest
from pathlib import Path

HELPER = Path(__file__).parents[1] / "src-tauri" / "src" / "monitter_mcp.py"

class Broker(http.server.BaseHTTPRequestHandler):
    calls = []
    def do_POST(self):
        length = int(self.headers["Content-Length"]); body = json.loads(self.rfile.read(length))
        Broker.calls.append((self.headers.get("Authorization"), body))
        if body["tool"] == "cancel_delegation": payload = {"error":{"message":"Cannot cancel completed task."}}
        else: payload = {"result":{"ok":True,"tool":body["tool"]}}
        encoded = json.dumps(payload).encode(); self.send_response(200); self.send_header("Content-Length", str(len(encoded))); self.end_headers(); self.wfile.write(encoded)
    def log_message(self, *_): pass

class MCPTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Broker)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
    def helper(self):
        env = os.environ | {"MONITTER_ENDPOINT":f"http://127.0.0.1:{self.server.server_port}/rpc", "MONITTER_TOKEN":"test-token"}
        return subprocess.Popen([sys.executable, str(HELPER)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    def rpc(self, process, value):
        process.stdin.write(json.dumps(value) + "\n"); process.stdin.flush(); return json.loads(process.stdout.readline())
    def test_handshake_list_forward_and_error(self):
        process = self.helper()
        try:
            initialized = self.rpc(process, {"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
            self.assertIn("list_agents", initialized["result"]["instructions"])
            self.assertLessEqual(len(initialized["result"]["instructions"]), 512)
            listing = self.rpc(process, {"jsonrpc":"2.0","id":2,"method":"tools/list"})
            self.assertEqual({tool["name"] for tool in listing["result"]["tools"]}, {"list_agents","delegate_task","send_message","get_task_result","wait_for_task","list_messages","cancel_delegation","terminal_run","skills_help","list_shared_skills","install_shared_skill"})
            install = next(tool for tool in listing["result"]["tools"] if tool["name"] == "install_shared_skill")
            self.assertFalse(install["annotations"]["readOnlyHint"])
            self.assertEqual(install["inputSchema"]["required"], ["url"])
            forwarded_install = self.rpc(process, {"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"install_shared_skill","arguments":{"url":"https://example.com/SKILL.md"}}})
            self.assertFalse(forwarded_install["result"].get("isError", False))
            self.assertEqual(Broker.calls[-1][1]["arguments"], {"url":"https://example.com/SKILL.md"})
            invalid_install = self.rpc(process, {"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"install_shared_skill","arguments":{"url":123}}})
            self.assertIn("error", invalid_install)
            wait = next(tool for tool in listing["result"]["tools"] if tool["name"] == "wait_for_task")
            self.assertFalse(wait["annotations"]["readOnlyHint"])  # Waiting can acknowledge inbox delivery.
            self.assertFalse(wait["inputSchema"]["additionalProperties"])
            forwarded = self.rpc(process, {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_agents","arguments":{}}})
            self.assertIn('"ok":true', forwarded["result"]["content"][0]["text"])
            self.assertEqual(Broker.calls[-1][0], "Bearer test-token")
            failed = self.rpc(process, {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cancel_delegation","arguments":{"collaboration_id":"x"}}})
            self.assertTrue(failed["result"]["isError"])
            malformed = self.rpc(process, {"jsonrpc":"2.0","id":5,"method":"tools/call","params":[]})
            self.assertEqual(malformed["error"]["code"], -32600)
            unknown = self.rpc(process, {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_agents","arguments":{"caller_task_id":"forged"}}})
            self.assertEqual(unknown["error"]["code"], -32600)
        finally:
            process.terminate(); process.wait(timeout=5)
            process.stdin.close(); process.stdout.close(); process.stderr.close()

if __name__ == "__main__": unittest.main()
