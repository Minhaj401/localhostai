"""Regression checks for the retained legacy node (no live model required)."""
import json
import unittest
from unittest.mock import patch

import httpx

import server


class LegacyNodeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.old_client = server.client
        self.requests = []

        async def upstream(request):
            if request.url.path.endswith("/models"):
                return httpx.Response(200, json={"data": [{"id": "test-model"}]})
            self.requests.append(json.loads(request.content))
            return httpx.Response(200, json={"choices": [{"message": {"content": "ok"}}]})

        server.client = httpx.AsyncClient(transport=httpx.MockTransport(upstream))
        server._state.update(backend="ollama", base_url="http://engine/v1", models_cache=["test-model"])
        server._cluster.update(proc=None, peers=[], model=None)
        self.api = httpx.AsyncClient(transport=httpx.ASGITransport(app=server.app), base_url="http://node")

    async def asyncTearDown(self):
        await self.api.aclose()
        await server.client.aclose()
        server.client = self.old_client

    async def test_missing_model_uses_the_advertised_default(self):
        response = await self.api.post("/v1/chat/completions", json={
            "model": None, "messages": [{"role": "user", "content": "hi"}],
        })
        self.assertEqual(response.status_code, 200)
        self.assertEqual(self.requests[0]["model"], "test-model")

    async def test_non_object_requests_return_bad_request(self):
        for endpoint in ("/v1/chat/completions", "/cluster/start"):
            response = await self.api.post(endpoint, json=[])
            self.assertEqual(response.status_code, 400)

    async def test_stopping_is_successful_even_without_a_fallback_engine(self):
        server._state["backend"] = "distributed"
        with patch.object(server, "_pick_backend", side_effect=RuntimeError("no engine")):
            response = await self.api.post("/cluster/stop")
        self.assertEqual(response.status_code, 200)
        self.assertTrue(response.json()["ok"])
        self.assertEqual(server._state["models_cache"], [])

    def test_rpc_command_keeps_all_peers_and_enables_offloading(self):
        command = server._cluster_command("test.gguf", ["phone:50052", "other:50052"], 4096, 4, [])
        self.assertEqual(command.count("--rpc"), 1)
        self.assertEqual(command[command.index("--rpc") + 1], "phone:50052,other:50052")
        self.assertEqual(command[command.index("-ngl") + 1], "999")


if __name__ == "__main__":
    unittest.main()
