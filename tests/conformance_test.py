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
import hashlib
import sys
from pathlib import Path
from typing import Any

import mcp.types as types
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
from pydantic import TypeAdapter

# The Skills extension (io.modelcontextprotocol/skills, MCP 2026-07-28).
SKILLS_EXTENSION = "io.modelcontextprotocol/skills"
RAW_RESULT = TypeAdapter(dict[str, Any])


class UriParams(types.RequestParams):
    """`{uri}` — the one parameter `skills/get` and `resources/directory/read` take."""

    uri: str

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
            check("lists the playbooks but not their reference files",
                  not any(str(r.uri).startswith("skill://") and "/references/" in str(r.uri)
                          for r in resources))
            # The pre-extension catalog is deprecated: unlisted, still readable
            # for one release so an installed skill text naming it does not break.
            check("does not list the deprecated skill index",
                  not any(str(r.uri) == "skill://index.json" for r in resources))

            templates = (await session.list_resource_templates()).resource_templates
            check("returns resource templates", len(templates) > 0)

            prompts = (await session.list_prompts()).prompts
            check("returns a non-empty prompt list", len(prompts) > 0,
                  f"{len(prompts)} prompts")

            # A read that needs no daemon: the deprecated catalog is compiled in
            # and says what replaced it.
            index = await session.read_resource("skill://index.json")
            check("the deprecated skill index still reads and says so",
                  bool(index.contents) and "skills/list" in (index.contents[0].text or ""))

            print("\n[ skills extension ]")
            # The extension is declared on the handshake and on the stateless
            # 2026-07-28 `server/discover` alike — a client issues the skills
            # methods only after seeing it.
            caps = init.capabilities.model_dump(by_alias=True)
            ext = (caps.get("extensions") or {}).get(SKILLS_EXTENSION)
            check("initialize declares the skills extension", isinstance(ext, dict), str(caps))
            check("the extension declares directoryRead", (ext or {}).get("directoryRead") is True)
            # What a Claude client actually reads: the catalog in instructions.
            instructions = init.instructions or ""
            discover = await session.send_discover("2026-07-28")
            check("server/discover lists 2026-07-28",
                  "2026-07-28" in (discover.get("supportedVersions") or []))
            check("server/discover declares the skills extension",
                  isinstance(((discover.get("capabilities") or {}).get("extensions") or {})
                             .get(SKILLS_EXTENSION), dict))

            listing = await session.send_request(
                types.Request(method="skills/list", params={}), RAW_RESULT)
            skills = listing.get("skills") or []
            check("skills/list returns entries", len(skills) >= 5, f"{len(skills)} skills")
            check("instructions carry every skill's name and trigger description",
                  all(f"- {(sk.get('frontmatter') or {}).get('name')}: "
                      f"{(sk.get('frontmatter') or {}).get('description')}" in instructions
                      for sk in skills), instructions[:200])
            check("skills/list carries resultType, ttlMs and cacheScope",
                  listing.get("resultType") == "complete"
                  and isinstance(listing.get("ttlMs"), int)
                  and listing.get("cacheScope") in ("public", "private"))
            # Every entry: a SKILL.md URI whose last path segment is the
            # frontmatter name, and a complete manifest whose digests and sizes
            # describe exactly the bytes resources/read serves — the host's
            # verification, run here.
            bad_entries, bad_files, verified = [], [], 0
            for skill in skills:
                uri = skill.get("uri") or ""
                fm = skill.get("frontmatter") or {}
                manifest = skill.get("resources")
                if not (uri.endswith("/SKILL.md") and isinstance(manifest, list)
                        and uri.rsplit("/", 2)[-2] == fm.get("name") and fm.get("description")
                        and any(f.get("uri") == uri for f in manifest)):
                    bad_entries.append(uri)
                    continue
                for f in manifest:
                    body = (await session.read_resource(f["uri"])).contents[0].text.encode("utf-8")
                    if (f.get("size") != len(body)
                            or f.get("digest") != "sha256:" + hashlib.sha256(body).hexdigest()):
                        bad_files.append(f["uri"])
                    verified += 1
                one = await session.send_request(
                    types.Request(method="skills/get", params=UriParams(uri=uri)), RAW_RESULT)
                if one.get("skill") != skill or one.get("resultType") != "complete":
                    bad_entries.append(uri + " (skills/get disagrees)")
            check("every skill entry is well-formed and skills/get agrees",
                  not bad_entries, str(bad_entries))
            check(f"every manifest digest and size verifies ({verified} files)",
                  not bad_files, str(bad_files))

            root = await session.send_request(
                types.Request(method="resources/directory/read",
                              params=UriParams(uri=skills[0]["uri"].rsplit("/", 1)[0])),
                RAW_RESULT)
            children = root.get("resources") or []
            check("resources/directory/read lists the skill root",
                  any(c.get("name") == "SKILL.md" for c in children)
                  and any(c.get("mimeType") == "inode/directory" for c in children),
                  str(children)[:200])

            try:
                await session.send_request(
                    types.Request(method="skills/get",
                                  params=UriParams(uri="skill://does-not-exist/SKILL.md")),
                    RAW_RESULT)
                check("skills/get of an unknown skill is -32602", False, "it was accepted")
            except Exception as err:  # noqa: BLE001
                check("skills/get of an unknown skill is -32602",
                      getattr(err, "code", None) == -32602, str(err)[:160])

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
