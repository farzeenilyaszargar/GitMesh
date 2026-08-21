import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  encodeOptionalTextArg,
  profileFromFields,
  readJsonBody,
  requestDaemon,
  requireMutationAuth,
  safeAccountToken
} from "../../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

type RouteContext = {
  params: Promise<{ username: string }>;
};

export async function GET(_request: NextRequest, context: RouteContext) {
  const { username } = await context.params;
  if (!safeAccountToken(username)) {
    return NextResponse.json({ ok: false, error: "invalid username" }, { status: 400 });
  }
  const response = await requestDaemon(`ACCOUNT_PROFILE ${username}`);
  return NextResponse.json(
    response.ok ? { ...response, profile: profileFromFields(response.fields) } : response,
    { status: response.ok ? 200 : 503 }
  );
}

export async function PATCH(request: NextRequest, context: RouteContext) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }
  const { username } = await context.params;
  if (!safeAccountToken(username)) {
    return NextResponse.json({ ok: false, error: "invalid username" }, { status: 400 });
  }

  const body = await readJsonBody(request);
  const response = await requestDaemon(
    `ACCOUNT_UPDATE_PROFILE ${username} ${encodeOptionalTextArg(body.displayName)} ${encodeOptionalTextArg(body.bio)} ${encodeOptionalTextArg(body.avatarUri)}`,
    { admin: true }
  );
  return daemonJson(
    response.ok ? { ...response, profile: profileFromFields(response.fields) } : response
  );
}
