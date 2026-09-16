import type { Metadata } from "next";

import { Providers } from "./providers";
import { Navbar } from "./components/navbar";

import "./globals.css";

export const metadata: Metadata = {
  title: "Creditcoin Bridge",
  description:
    "Bridge testnet assets between Base Sepolia and Ethereum Sepolia via Creditcoin",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className="min-h-screen antialiased">
        <Providers>
          <Navbar />
          <main className="mx-auto max-w-3xl px-4 py-10">{children}</main>
        </Providers>
      </body>
    </html>
  );
}
