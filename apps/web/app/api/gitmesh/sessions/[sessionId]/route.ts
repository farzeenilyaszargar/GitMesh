import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  requestDaemon,
  requireMutationAuth,
  safeToken
} from "../../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

type RouteContext = {
  params: Promise<{ sessionId: string }>;
};

export async function DELETE(request: NextRequest, context: RouteContext) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }
  const { sessionId } = await context.params;
  if (!safeToken(sessionId)) {
    return NextResponse.json({ ok: false, error: "invalid session id" }, { status: 400 });
  }

  return daemonJson(
    await requestDaemon(`SESSION_REVOKE ${sessionId}`, {
      admin: true
    })
  );
}
