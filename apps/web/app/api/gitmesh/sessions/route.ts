import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  readJsonBody,
  requestDaemon,
  requireMutationAuth,
  safeAccountToken,
  safeToken
} from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(request: NextRequest) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }

  const body = await readJsonBody(request);
  const username = stringField(body.username);
  const ttlSeconds = numberOrStringField(body.ttlSeconds) ?? "86400";
  const deviceId = stringField(body.deviceId) ?? "none";

  if (
    !username ||
    !safeAccountToken(username) ||
    !safeToken(ttlSeconds) ||
    !safeToken(deviceId)
  ) {
    return NextResponse.json(
      { ok: false, error: "username, ttlSeconds, or deviceId is invalid" },
      { status: 400 }
    );
  }

  return daemonJson(
    await requestDaemon(`SESSION_ISSUE ${username} ${ttlSeconds} ${deviceId}`, {
      admin: true
    }),
    201
  );
}

function stringField(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

function numberOrStringField(value: unknown) {
  if (typeof value === "number" && Number.isInteger(value) && value > 0) {
    return value.toString();
  }
  if (typeof value === "string") {
    return value;
  }
  return undefined;
}
