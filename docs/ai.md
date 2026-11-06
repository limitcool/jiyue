# AI (bring your own key)

Jiyue's AI features are optional, off by default, and use a provider account that you own. Jiyue has no AI service of its own, and it sends nothing to any server until you enable AI and ask for something.

## Providers

Two kinds of provider are supported, and you add as many as you like in the AI settings:

| Kind | Works with |
| --- | --- |
| OpenAI-compatible | Any service that implements the OpenAI chat and embeddings API. You enter its base URL (it must start with `https://`, or `http://` for a local server) and the model names. |
| Anthropic | The Anthropic Messages API. |

There are no built-in presets: enter the base URL and model names that your provider documents. A chat provider is required. An embedding provider is optional; without one, search for the question uses only the keyword index.

## Setup

1. Open Settings (Ctrl+,) and go to **AI**.
2. Turn on **启用 AI 功能**.
3. Add a chat provider: name, kind, base URL, chat model and the API key. Optionally add an embedding provider with an embedding model.
4. Choose excluded folders and tags if some notes must never leave your machine.

## Where things are stored

- The API key is kept in plain text in `credentials.toml` in your user configuration folder, under `[ai.<provider id>]`. It is readable only by your user account, never written into the vault, never synced and never written to a log. You can write `env:NAME` or `cmd:COMMAND` instead of the key itself. See [configuration.md](configuration.md).
- The `[ai]` section of `.jiyue/config.toml` in the vault holds everything else: enabled flag, providers (name, kind, base URL, models), excluded folders and tags. It contains no secret.
- Vector index for embeddings: `.jiyue/index/`. Notes are split into chunks of about 800 characters, and each chunk is sent once to the embedding provider, which bills it according to its own prices. The index can be deleted at any time; it is rebuilt from the notes when needed.

## What is sent, and when

- Nothing is sent unless AI is enabled, and only when you ask for it.
- Excluded folders and excluded tags are never sent. The check applies to the active note and to notes found for a vault-wide question.
- **Questions** (the side chat) show a preview of the text that will be sent (the chosen note, the selected text, and the search results) before anything leaves the app. You confirm first.
- **Actions** (for example rewriting the selection, summarising the note, suggesting a title or tags) are started by you from the menu. The result appears as a diff preview; you accept or reject it. Nothing is written into the note before you accept.
- Embedding requests send the note chunks, when you use an embedding provider and the index is updated.

## AI actions

- **Side panel** (Ctrl+Shift+A): chat about the current note, the selection, or the whole vault. Answers stream in as they arrive.
- **Selection menu** (Ctrl+J): rewrite, shorten, expand, translate, fix, summarise.
- **Note actions** (command palette, "AI："): summarise the current note, suggest a title, suggest tags, ask the whole vault.

Requests have a connection timeout, an idle timeout and retries with back-off. A long `Retry-After` from the provider is not waited for; the error is shown instead. Background indexing can be cancelled.

## Limits

- The quality and the cost of answers depend on the provider and the model you choose.
- Jiyue does not check whether the provider stores or trains on what it receives. Read your provider's terms.
- A model server on your own machine (for example an OpenAI-compatible server on `http://localhost`) keeps the text local. Any other address sends it over the network.
