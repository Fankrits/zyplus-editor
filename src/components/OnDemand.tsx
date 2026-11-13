import { Suspense, useState, type ReactNode } from "react";

/**
 * Mounts `children` (normally a lazy component) the first time `when` is true, and keeps them mounted
 * after, so closing still animates. Until then nothing is rendered and the lazy chunk is never fetched:
 * a modal, menu or drawer that most sessions never open costs no JavaScript in the page.
 */
export function OnDemand({ when, children }: { when: boolean; children: ReactNode }) {
  const [wanted, setWanted] = useState(when);
  if (when && !wanted) setWanted(true);
  return wanted ? <Suspense fallback={null}>{children}</Suspense> : null;
}
