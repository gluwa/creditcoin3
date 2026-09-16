"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { ConnectButton } from "@rainbow-me/rainbowkit";

import { cn } from "@/lib/utils";

const LINKS = [
  { href: "/deposit", label: "Deposit" },
  { href: "/history", label: "History" },
  { href: "/inspect", label: "Inspect" },
];

export function Navbar() {
  const pathname = usePathname();

  return (
    <header className="border-b">
      <div className="mx-auto flex max-w-3xl items-center justify-between px-4 py-4">
        <div className="flex items-center gap-6">
          <span className="font-semibold">Creditcoin Bridge</span>
          <nav className="flex items-center gap-4 text-sm">
            {LINKS.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                className={cn(
                  "text-muted-foreground transition-colors hover:text-foreground",
                  pathname.startsWith(link.href) &&
                    "font-medium text-foreground",
                )}
              >
                {link.label}
              </Link>
            ))}
          </nav>
        </div>
        <ConnectButton showBalance={false} />
      </div>
    </header>
  );
}
