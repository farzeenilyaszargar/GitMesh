import type { Metadata } from "next";
import "./globals.css";
import { FooterShell } from "./footer-shell";

export const metadata: Metadata = {
  title: "GitMesh",
  description:
    "A dark premium Git hosting interface for decentralized repositories.",
  icons: {
    icon: "/gitmesh-logo-white.png",
    apple: "/gitmesh-logo-white.png"
  }
};

export default function RootLayout({
  children
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body>
        <div className="pageFrame">{children}</div>
        <FooterShell />
      </body>
    </html>
  );
}
