/** Written into the default folder on first run and opened as the first tab. */
export const WELCOME_NOTE_NAME = "Welcome.md";

export const WELCOME_NOTE = `# Welcome to Zyplus

**A markdown editor that gets out of the way.** This note lives in your Zyplus folder — edit it, rename it, or delete it. It won't come back.

> [!TIP]
> Press \`⌘E\` (\`Ctrl+E\` on Windows and Linux) to flip this tab between rich text and raw markdown.

## The basics

- **Your notes are plain files.** Everything is a \`.md\` file in a real folder on disk — no database, no lock-in.
- **New files land here.** This folder is your default; change it in Settings (\`⌘,\`).
- **Open anything.** Add more project folders with \`⌘O\`, or open a single file with \`⌘⇧O\`.
- **Pick up where you left off.** Open folders and tabs come back exactly as you left them.

## Beyond plain text

Callouts and syntax-highlighted code work out of the box. Diagrams and math render once you turn on **Mermaid** and **KaTeX** in Settings → Extensions:

\`\`\`mermaid
flowchart LR
    A[Write] --> B[Save]
    B --> C[Sync]
\`\`\`

\`\`\`latex
e^{i\\pi} + 1 = 0
\`\`\`

## Handy shortcuts

| Action | Shortcut |
|---|---|
| New file | \`⌘N\` |
| Save | \`⌘S\` |
| Toggle rich / plain | \`⌘E\` |
| Find | \`⌘F\` |
| Toggle sidebar | \`⌘B\` |
| Export as PDF | \`⌘P\` |
| All shortcuts | \`⌘/\` |

Sign in from Settings to sync this folder across your devices. Happy writing!
`;
