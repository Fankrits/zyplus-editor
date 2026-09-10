import { ALLOWED_ORIGINS, auth, userIdFrom } from "./auth";
import { getContent, isValidRelPath, listSince, put, softDelete } from "./notes";

const PORT = Number(process.env.PORT ?? 3000);

/**
 * `set` rather than `append`, so these win even if a downstream handler set its
 * own — duplicate `Access-Control-Allow-Origin` headers fail the preflight.
 */
function withCors(res: Response, origin: string | null): Response {
  if (origin && ALLOWED_ORIGINS.includes(origin)) {
    res.headers.set("Access-Control-Allow-Origin", origin);
    res.headers.set("Vary", "Origin");
    res.headers.set("Access-Control-Allow-Headers", "Content-Type, Authorization");
    res.headers.set("Access-Control-Allow-Methods", "GET, PUT, POST, DELETE, OPTIONS");
    res.headers.set("Access-Control-Expose-Headers", "set-auth-token");
  }
  return res;
}

function json(body: unknown, status = 200): Response {
  return Response.json(body, { status });
}

/** Wraps a handler with CORS, preflight, and a 500 that never leaks internals. */
function route(handler: (req: Request) => Promise<Response> | Response) {
  return async (req: Request): Promise<Response> => {
    const origin = req.headers.get("origin");
    // One line per request: without it, a client that cannot reach the server at
    // all looks exactly like one whose request was rejected.
    console.log(`${req.method} ${new URL(req.url).pathname} origin=${origin ?? "-"}`);
    if (req.method === "OPTIONS") return withCors(new Response(null, { status: 204 }), origin);
    try {
      return withCors(await handler(req), origin);
    } catch (err) {
      console.error(`${req.method} ${new URL(req.url).pathname} failed:`, err);
      return withCors(json({ error: "Internal error" }, 500), origin);
    }
  };
}

/** Resolves the caller, or returns the 401 to send back. */
async function requireUser(req: Request): Promise<{ userId: string } | { res: Response }> {
  const userId = await userIdFrom(req);
  if (!userId) return { res: json({ error: "Unauthorized" }, 401) };
  return { userId };
}

/** The path a note request names, or the 400 to send back. */
function requirePath(value: string | null): { relPath: string } | { res: Response } {
  if (!isValidRelPath(value)) return { res: json({ error: "Invalid path" }, 400) };
  return { relPath: value };
}

const server = Bun.serve({
  port: PORT,
  routes: {
    "/api/auth/*": route((req) => auth.handler(req)),

    "/api/notes": route(async (req) => {
      const caller = await requireUser(req);
      if ("res" in caller) return caller.res;
      const url = new URL(req.url);

      if (req.method === "GET") {
        const since = Number(url.searchParams.get("since") ?? 0);
        if (!Number.isFinite(since) || since < 0) return json({ error: "Invalid since" }, 400);
        return json({ notes: await listSince(caller.userId, since) });
      }

      if (req.method === "PUT") {
        const body = (await req.json().catch(() => null)) as {
          relPath?: unknown;
          content?: unknown;
          baseRev?: unknown;
        } | null;
        if (!body) return json({ error: "Invalid body" }, 400);

        const path = requirePath(typeof body.relPath === "string" ? body.relPath : null);
        if ("res" in path) return path.res;
        if (typeof body.content !== "string") return json({ error: "Invalid content" }, 400);
        const baseRev = Number(body.baseRev ?? 0);
        if (!Number.isFinite(baseRev) || baseRev < 0) return json({ error: "Invalid baseRev" }, 400);

        const result = await put(caller.userId, path.relPath, body.content, baseRev);
        return "conflict" in result
          ? json({ rev: result.rev, content: result.content }, 409)
          : json(result);
      }

      if (req.method === "DELETE") {
        const path = requirePath(url.searchParams.get("path"));
        if ("res" in path) return path.res;
        const result = await softDelete(caller.userId, path.relPath);
        return json(result ?? { rev: 0 });
      }

      return json({ error: "Method not allowed" }, 405);
    }),

    "/api/notes/content": route(async (req) => {
      if (req.method !== "GET") return json({ error: "Method not allowed" }, 405);
      const caller = await requireUser(req);
      if ("res" in caller) return caller.res;

      const path = requirePath(new URL(req.url).searchParams.get("path"));
      if ("res" in path) return path.res;

      const note = await getContent(caller.userId, path.relPath);
      return note ? json(note) : json({ error: "Not found" }, 404);
    }),

    "/health": route(() => json({ ok: true })),
  },

  fetch: () => new Response("Not found", { status: 404 }),
});

console.log(`Zyplus API on http://localhost:${server.port}`);
