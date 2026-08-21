import { NextResponse } from "next/server";

import { requestDaemon } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  const status = await requestDaemon("REPO_STATUS");
  return NextResponse.json(status, { status: status.ok ? 200 : 503 });
}
