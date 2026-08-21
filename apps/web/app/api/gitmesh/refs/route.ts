import { NextResponse } from "next/server";

import { parseRefs, requestDaemon } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  const response = await requestDaemon("REF_LIST");
  return NextResponse.json(
    {
      ...response,
      refs: parseRefs(response.fields.refs)
    },
    { status: response.ok ? 200 : 503 }
  );
}
