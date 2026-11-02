---
title: "Install the MCP server in an agent host"
description: "Add the bacnet CLI's MCP server to Claude Desktop with its bundle, or to Claude Code and other agent hosts with the plain command, after authorizing a profile."
---

The [MCP server](/rusty-bacnet/guides/ai-agents/) is part of the `bacnet` CLI, so installing it means telling an agent host how to start `bacnet mcp serve --stdio`. Claude Desktop installs it from an MCP bundle: one file, downloaded from the GitHub release, that holds the `bacnet` executable and the settings form. Claude Code and other hosts start the CLI you already installed. Either way the server is **read-only** unless the profile it runs on offers writes, and it starts only on a profile a person has authorized.

## First, authorize a profile

The server connects through a [TUI profile](/rusty-bacnet/tui/profiles/): its transport settings say how to reach the site, and its `[mcp]` table limits what an agent may reach there. An agent host can't show the authorization question, so a person answers it once in a terminal, before the host starts the server:

1. Create the profile with `bacnet tui`, or write it by hand in the profiles folder of the [config directory](/rusty-bacnet/tui/profiles/#where-profiles-live).
2. Add an `[mcp]` table if the agent should reach less than the whole site ([example](/rusty-bacnet/guides/ai-agents/#authorize-a-profile)).
3. Authorize it:

   ```sh
   bacnet mcp authorize --profile plant-a
   ```

Changing the profile's transport, or widening its `[mcp]` table, clears the authorization; run the command again, then restart the server in the host.

An agent host starts the server with its own environment, not your shell's. A `BACNET_CONFIG_DIR` set in your shell profile doesn't reach it, so the server looks in the default config directory unless the host's configuration sets the variable (Claude Desktop's bundle can't).

## Claude Desktop

Releases after 0.15.0 attach an MCP bundle (`.mcpb`) for each platform beside the `bacnet` executables. The bundle format names an operating system but not a processor, so pick the file for your computer:

| Computer | Bundle |
|---|---|
| Mac with Apple silicon | `bacnet-macos-arm64.mcpb` |
| Mac with an Intel processor | `bacnet-macos-amd64.mcpb` |
| Windows on x86-64 | `bacnet-windows-amd64.mcpb` |
| Linux on x86-64 | `bacnet-linux-amd64.mcpb` |
| Linux on 64-bit Arm | `bacnet-linux-arm64.mcpb` |

On Linux, use a bundle only with a host that installs MCP bundles there; otherwise set the host up as under [Other agent hosts](#other-agent-hosts).

1. Download the bundle from the [GitHub release](https://github.com/jscott3201/rusty-bacnet/releases) and check it against the release's `SHA256SUMS`, as the [installation page](/rusty-bacnet/start/installation/) shows for the executables. The bundles aren't signed.
2. Open the file with Claude Desktop, for example by double-clicking it, and install the extension it shows.
3. Enter the name of the profile you authorized, such as `plant-a`, in the extension's **Profile** setting. That is its only setting: the profile holds the transport, the reach and the write policy.

The bundle runs its own copy of `bacnet`, the same executable as the release's download for that platform. To update, install the next release's bundle; the authorization lives in the profile, so it carries over.

If the server doesn't start, Claude Desktop shows that it failed, and its log for the extension has the server's message. A profile that isn't there names the folder it was looked for in; a profile that isn't authorized, or no longer is, names the command to run. Both stop the server at once, before it reads anything from the host.

## Claude Code

Claude Code starts the CLI from your `PATH`. Add the server for the current project with `claude mcp add`, or in the project's `.mcp.json` as [Use rusty-bacnet with AI agents](/rusty-bacnet/guides/ai-agents/#add-it-to-the-agent-host) shows:

```sh
claude mcp add rusty-bacnet -- bacnet mcp serve --stdio --profile plant-a
```

## Other agent hosts

Any host that starts a local MCP server over stdio can run the same command. Most take a JSON entry like this one; give the full path of `bacnet` if the host doesn't see your shell's `PATH`, as desktop apps often don't:

```json
{
  "mcpServers": {
    "bacnet": {
      "command": "bacnet",
      "args": ["mcp", "serve", "--stdio", "--profile", "plant-a"]
    }
  }
}
```

A host that lets you set the server's environment can also pass `BACNET_CONFIG_DIR` there, for a profile store outside the default directory.

Without a profile, only the passive server runs: it sends nothing and offers the frame decoder and an empty discovery table. The bundle doesn't offer it, since it always runs a profile; add it to a host by hand when decoding captured frames is all you need:

```sh
bacnet mcp serve --stdio --passive
```
