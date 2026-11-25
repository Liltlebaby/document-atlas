Document Atlas ISO Q&A runtime

Uses @roberttlange/headless 0.6.1 (Apache-2.0) to build the Codex invocation and extract the native final answer.
Source: https://github.com/RobertTLange/headless-cli

The adapter removes Headless's default --search flag, disables inherited user config, shell tools, plugins, apps and memories, uses read-only sandboxing and ephemeral sessions, fixes the model to gpt-6-luna, and requires ChatGPT login. API-key environment variables are removed from the child environment. It does not fall back to API billing or another model.

Requires Node.js 22+ and a locally installed Codex CLI logged in with ChatGPT. These are already present on the development computer.
Node modules are installed with the included package-lock.json. Do not run the bridge against an untrusted/public endpoint.
