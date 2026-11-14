// Renders the package's real components in a test: Vite loads the .tsx
// source as the app builds it, and React renders it to markup. Not a test
// file itself (node --test runs *.test.ts only).
import { createServer, type ViteDevServer } from "vite";

let server: Promise<ViteDevServer> | null = null;

/** The module at `path` (from the package root, such as
 * "/src/sessions/SessionRow.tsx"), loaded as the app loads it. */
export async function load<T>(path: string): Promise<T> {
  server ??= createServer({
    root: new URL("..", import.meta.url).pathname,
    configFile: false,
    logLevel: "silent",
    appType: "custom",
    server: { middlewareMode: true, hmr: false, ws: false },
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  return (await (await server).ssrLoadModule(path)) as T;
}

export async function close() {
  if (server) await (await server).close();
  server = null;
}

export interface Tag {
  /** The words drawn in the tag. */
  shown: string;
  /** Its hover title. */
  title: string | null;
  /** What a screen reader reads: its text outside aria-hidden parts. */
  name: string;
}

/** Every machine tag in `markup`, as a viewer and a screen reader get it. */
export function machineTags(markup: string): Tag[] {
  const tags: Tag[] = [];
  const token = /<(\/?)([a-zA-Z][\w-]*)([^>]*?)(\/?)>|([^<]+)/g;
  let open: {
    depth: number;
    hidden: number;
    tag: Tag;
  } | null = null;
  let depth = 0;
  for (const match of markup.matchAll(token)) {
    const [, closing, , attributes = "", selfClosing, text] = match;
    if (text !== undefined) {
      if (open) {
        const words = decode(text);
        if (open.hidden > 0 && open.hidden <= depth) open.tag.shown += words;
        else open.tag.name += words;
      }
      continue;
    }
    if (closing) {
      if (open && open.hidden === depth) open.hidden = 0;
      depth--;
      if (open && depth < open.depth) {
        tags.push(open.tag);
        open = null;
      }
      continue;
    }
    if (selfClosing) continue;
    depth++;
    if (!open && /class="machine-label"/.test(attributes)) {
      const title = attributes.match(/title="([^"]*)"/)?.[1];
      open = {
        depth,
        hidden: 0,
        tag: {
          shown: "",
          title: title === undefined ? null : decode(title),
          name: "",
        },
      };
    } else if (
      open &&
      open.hidden === 0 &&
      /aria-hidden="true"/.test(attributes)
    ) {
      open.hidden = depth;
    }
  }
  return tags;
}

function decode(text: string) {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&#x27;|&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}
