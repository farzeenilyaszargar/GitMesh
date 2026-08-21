import { NextResponse } from "next/server";

import { requestDaemon } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  const health = await requestDaemon("PING");
  return NextResponse.json(health, { status: health.ok ? 200 : 503 });
}
