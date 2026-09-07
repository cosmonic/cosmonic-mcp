#!/usr/bin/env python3
"""Independent conformance check, driven by the OFFICIAL Python MCP SDK.

Why a second client at all: this crate's own tests, and the e2e in
cosmonic/desktop, are written against the same assumptions the crate is. They
cannot catch a mistake that is *consistent* with those assumptions — a schema
shape every client but ours rejects, a field the Rust SDK serialises in a way
only the Rust SDK reads back. So this drives the server with an implementation
that shares nothing with it: a different language, a different SDK, a different
JSON-RPC stack.

That is not hypothetical. The MCP Inspector found 17 schema-portability
problems here that a hand-rolled driver had passed clean, two of them freshly
introduced by the audit that wrote the driver.

Run:  python tests/conformance_test.py [path/to/cosmonic-mcp]
Needs no daemon — every assertion below is about the protocol surface, which
the server answers whether or not cosmonicd is up.
"""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

FAILURES: list[str] = []
CHECKS = 0


def check(name: str, ok: bool, detail: str = "") -> None:
    global CHECKS
    CHECKS += 1
    if ok:
        print(f"  PASS  {name}")
    else:
        print(f"  FAIL  {name}" + (f" — {detail}" if detail else ""))
        FAILURES.append(name)


def find_binary() -> Path:
    if len(sys.argv) > 1:
        return Path(sys.argv[1]).resolve()
    root = Path(__file__).resolve().parent.parent
    for profile in ("release", "debug"):
        candidate = root / "target" / profile / "cosmonic-mcp"
        if candidate.exists():
            return candidate
    sys.exit("cosmonic-mcp not built — `cargo build --release --bin cosmonic-mcp`")


async def main() -> int:
    binary = find_binary()
    print(f"MCP conformance (official Python SDK) — {binary}\n")
    params = StdioServerParameters(command=str(binary), args=[])

    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            init = await session.initialize()

            print("[ handshake ]")
            check("server identifies itself, not the SDK",
                  init.server_info.name not in ("", "rmcp", None),
                  f"server_info.name={init.server_info.name!r}")
            check("advertises a protocol version", bool(init.protocol_version))
            check("declares tools, resources and prompts",
                  all([init.capabilities.tools, init.capabilities.resources,
                       init.capabilities.prompts]))
            check("sends instructions", bool(init.instructions))

            print("\n[ tools ]")
            tools = (await session.list_tools()).tools
            check("returns a non-empty tool list", len(tools) > 0, f"{len(tools)} tools")

            # Every tool the directory criteria care about, checked by a client
            # that had no hand in building them.
            unnamed = [t.name for t in tools if len(t.name) > 64]
            check("every tool name is <= 64 characters", not unnamed, str(unnamed))

            undescribed = [t.name for t in tools if not t.description]
            check("every tool has a description", not undescribed, str(undescribed))

            missing_ann = [t.name for t in tools if t.annotations is None]
            check("every tool carries annotations", not missing_ann, str(missing_ann))

            no_title = [t.name for t in tools
                        if not (t.annotations and t.annotations.title)]
            check("every tool carries a title", not no_title, str(no_title))

            def hints(t) -> int:
                a = t.annotations
                return sum(1 for v in (a.read_only_hint, a.destructive_hint) if v)

            bad_hints = [t.name for t in tools if hints(t) != 1]
            check("every tool declares exactly one of readOnly/destructive",
                  not bad_hints, str(bad_hints))

            selectors = {"action", "method", "operation", "verb", "command"}
            multiplexed = [t.name for t in tools
                           if selectors & set((t.input_schema or {}).get("properties", {}))]
            check("no tool multiplexes reads and writes behind a selector",
                  not multiplexed, str(multiplexed))

            print("\n[ schema portability ]")
            # `{"type": ["string","null"]}` is legal JSON Schema and is what
            # schemars emits for Option<T>, but several MCP clients read `type`
            # as a single string and reject the tool or drop the constraint.
            array_types: list[str] = []
            typeless: list[str] = []

            def walk(node, path: str) -> None:
                if isinstance(node, dict):
                    if isinstance(node.get("type"), list):
                        array_types.append(path)
                    for key, value in node.items():
                        walk(value, f"{path}.{key}")
                elif isinstance(node, list):
                    for i, value in enumerate(node):
                        walk(value, f"{path}[{i}]")

            for tool in tools:
                schema = tool.input_schema or {}
                walk(schema, tool.name)
                for prop, sub in (schema.get("properties") or {}).items():
                    if isinstance(sub, dict) and not (
                        {"type", "anyOf", "oneOf", "$ref", "enum", "const"} & set(sub)
                    ):
                        typeless.append(f"{tool.name}.{prop}")

            check("no schema uses the array `type` form", not array_types, str(array_types))
            check("every input property declares a type", not typeless, str(typeless))

            print("\n[ resources and prompts ]")
            resources = (await session.list_resources()).resources
            check("returns a non-empty resource list", len(resources) > 0,
                  f"{len(resources)} resources")
            check("publishes the skill index",
                  any(str(r.uri) == "skill://index.json" for r in resources))

            templates = (await session.list_resource_templates()).resource_templates
            check("returns resource templates", len(templates) > 0)

            prompts = (await session.list_prompts()).prompts
            check("returns a non-empty prompt list", len(prompts) > 0,
                  f"{len(prompts)} prompts")

            # A read that needs no daemon: the skill catalog is compiled in.
            index = await session.read_resource("skill://index.json")
            check("reads the skill index", bool(index.contents))

            print("\n[ error quality ]")
            # The criteria reject generic errors. An unknown resource must say
            # what was missing.
            try:
                await session.read_resource("cosmonic://does-not-exist")
                check("unknown resource is rejected", False, "it was accepted")
            except Exception as err:  # noqa: BLE001 — any refusal is fine
                text = str(err)
                check("unknown resource is rejected with detail",
                      "does-not-exist" in text, text[:160])

    print(f"\n{CHECKS - len(FAILURES)} passed, {len(FAILURES)} failed")
    return 1 if FAILURES else 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
