"use client";

import { usePathname } from "next/navigation";
import { SiteFooter } from "./site-footer";

export function FooterShell() {
  const pathname = usePathname();

  if (pathname === "/" || pathname === "/login") {
    return null;
  }

  return <SiteFooter />;
}
