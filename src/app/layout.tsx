import type { Metadata } from "next";

import { AppShell } from "@/components/layout/app-shell";
import { Providers } from "@/components/providers";

import "./globals.css";

export const metadata: Metadata = {
  title: "gazooo",
  description: "Local image generation desktop app",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="ja" className="h-full antialiased font-sans" suppressHydrationWarning>
      <body className="h-full">
        <Providers>
          <AppShell>{children}</AppShell>
        </Providers>
      </body>
    </html>
  );
}
