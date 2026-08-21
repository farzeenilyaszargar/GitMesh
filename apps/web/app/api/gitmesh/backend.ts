import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { NextRequest, NextResponse } from "next/server";

export type DaemonStatus = {
  ok: boolean;
  raw: string;
  fields: Record<string, string>;
  socketPath: string;
  error?: string;
};

export type GatewaySnapshot = {
  health: DaemonStatus;
  repo: DaemonStatus;
  refs: DaemonStatus;
  keyGrants: DaemonStatus;
};

const COMMAND_TIMEOUT_MS = 1200;
const DEFAULT_REPO_ID = "repo:farzeen/gitmesh";

type RequestDaemonOptions = {
  admin?: boolean;
};

export function daemonSocketPath() {
  return process.env.GITMESHD_SOCKET ?? path.join(tmpdir(), "gitmeshd.sock");
}

export async function requestDaemon(
  command: string,
  options: RequestDaemonOptions = {}
): Promise<DaemonStatus> {
  const socketPath = daemonSocketPath();
  const daemonCommand = options.admin ? adminWrap(command) : command;
  try {
    const raw = await sendLine(socketPath, `${daemonCommand}\n`);
    const ok = raw.startsWith("OK ");
    return {
      ok,
      raw,
      fields: ok ? parseFields(raw.slice(3)) : {},
      socketPath,
      error: ok ? undefined : raw
    };
  } catch (error) {
    return {
      ok: false,
      raw: "",
      fields: {},
      socketPath,
      error: error instanceof Error ? error.message : "daemon request failed"
    };
  }
}

export function daemonJson<T extends { ok: boolean }>(response: T, successStatus = 200) {
  return NextResponse.json(response, { status: response.ok ? successStatus : 503 });
}

export function requireMutationAuth(request: NextRequest) {
  const expected = process.env.GITMESH_WEB_ADMIN_TOKEN;
  if (!expected) {
    return null;
  }
  const actual = request.headers.get("x-gitmesh-admin-token");
  if (actual === expected) {
    return null;
  }
  return NextResponse.json(
    { ok: false, error: "mutation requires x-gitmesh-admin-token" },
    { status: 401 }
  );
}

export function safeToken(value: string) {
  return value.length > 0 && !/\s/.test(value);
}

export function safeAccountToken(value: string) {
  return /^[A-Za-z0-9._-]+$/.test(value);
}

export function encodeTextArg(value: string | undefined) {
  if (!value) {
    return "-";
  }
  return Buffer.from(value, "utf8").toString("hex");
}

export function encodeOptionalTextArg(value: unknown) {
  if (typeof value !== "string") {
    return "keep";
  }
  return encodeTextArg(value);
}

export function decodeHexField(value: string | undefined) {
  if (!value) {
    return "";
  }
  return Buffer.from(value, "hex").toString("utf8");
}

export function profileFromFields(fields: Record<string, string>) {
  return {
    username: fields.username,
    account: fields.account,
    displayName: decodeHexField(fields.display_hex),
    bio: decodeHexField(fields.bio_hex),
    avatarUri: decodeHexField(fields.avatar_hex),
    createdAt: fields.created_at ? Number(fields.created_at) : null,
    updatedAt: fields.updated_at ? Number(fields.updated_at) : null
  };
}

export async function readJsonBody(request: NextRequest) {
  try {
    return (await request.json()) as Record<string, unknown>;
  } catch {
    return {};
  }
}

export async function gatewaySnapshot(repoId = DEFAULT_REPO_ID): Promise<GatewaySnapshot> {
  const [health, repo, refs, keyGrants] = await Promise.all([
    requestDaemon("PING"),
    requestDaemon("REPO_STATUS"),
    requestDaemon("REF_LIST"),
    requestDaemon(`KEY_GRANT_STATUS ${repoId}`)
  ]);

  return { health, repo, refs, keyGrants };
}

export function parseRefs(value: string | undefined) {
  if (!value || value === "none") {
    return [];
  }
  return value.split(",").map((entry) => {
    const [name, oid] = entry.split(":");
    return { name, oid };
  });
}

function adminWrap(command: string) {
  const token = process.env.GITMESHD_ADMIN_TOKEN;
  return token ? `AUTH ${token} ${command}` : command;
}

function sendLine(socketPath: string, line: string) {
  return new Promise<string>((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    let settled = false;
    let response = "";
    const timer = setTimeout(() => {
      finish(new Error("gitmeshd request timed out"));
      socket.destroy();
    }, COMMAND_TIMEOUT_MS);

    function finish(error?: Error) {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      if (error) {
        reject(error);
      } else {
        resolve(response.trimEnd());
      }
    }

    socket.setEncoding("utf8");
    socket.on("connect", () => socket.write(line));
    socket.on("data", (chunk) => {
      response += chunk;
      if (response.includes("\n")) {
        socket.end();
        finish();
      }
    });
    socket.on("error", finish);
    socket.on("end", () => finish());
  });
}

function parseFields(message: string) {
  return Object.fromEntries(
    message
      .split(/\s+/)
      .filter(Boolean)
      .map((part) => {
        const index = part.indexOf("=");
        if (index === -1) {
          return [part, "true"];
        }
        return [part.slice(0, index), part.slice(index + 1)];
      })
  );
}
