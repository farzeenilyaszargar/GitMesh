import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  encodeTextArg,
  profileFromFields,
  readJsonBody,
  requestDaemon,
  requireMutationAuth,
  safeAccountToken,
  safeToken
} from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  return daemonJson(await requestDaemon("ACCOUNT_STATUS"));
}

export async function POST(request: NextRequest) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }

  const body = await readJsonBody(request);
  const username = stringField(body.username);
  const accountCid = stringField(body.accountCid);
  const displayName = stringField(body.displayName) ?? username;
  const bio = stringField(body.bio) ?? "";
  const avatarUri = stringField(body.avatarUri) ?? "";

  if (!username || !accountCid || !safeAccountToken(username) || !safeToken(accountCid)) {
    return NextResponse.json(
      { ok: false, error: "username and accountCid are required safe tokens" },
      { status: 400 }
    );
  }

  const response = await requestDaemon(
    `ACCOUNT_CREATE ${username} ${accountCid} ${encodeTextArg(displayName)} ${encodeTextArg(bio)} ${encodeTextArg(avatarUri)}`,
    { admin: true }
  );
  return NextResponse.json(
    response.ok ? { ...response, profile: profileFromFields(response.fields) } : response,
    { status: response.ok ? 201 : 503 }
  );
}

function stringField(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
