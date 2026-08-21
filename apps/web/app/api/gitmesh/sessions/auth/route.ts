import { NextRequest, NextResponse } from "next/server";

import { daemonJson, readJsonBody, requestDaemon, safeToken } from "../../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function POST(request: NextRequest) {
  const body = await readJsonBody(request);
  const token = typeof body.token === "string" ? body.token : "";

  if (!safeToken(token)) {
    return NextResponse.json({ ok: false, error: "token is required" }, { status: 400 });
  }

  return daemonJson(await requestDaemon(`SESSION_AUTH ${token}`));
}
