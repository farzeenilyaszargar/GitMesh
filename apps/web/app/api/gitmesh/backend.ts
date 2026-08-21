import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

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

export function daemonSocketPath() {
  return process.env.GITMESHD_SOCKET ?? path.join(tmpdir(), "gitmeshd.sock");
}

export async function requestDaemon(command: string): Promise<DaemonStatus> {
  const socketPath = daemonSocketPath();
  try {
    const raw = await sendLine(socketPath, `${command}\n`);
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
