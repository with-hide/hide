import type { ReactNode } from "react";
import { LaptopIcon } from "lucide-react";

/** The QR and version screens share the chosen device-first layout. */
export function Landing({ children, note }: { children: ReactNode; note?: ReactNode }) {
  return (
    <main className="phone-safe-top phone-safe-bottom flex min-h-full flex-col px-xl">
      <div className="pt-lg text-headline font-semibold text-foreground">hide</div>
      <div className="flex flex-1 flex-col items-center justify-center gap-lg py-xxxl text-center">{children}</div>
      {note ? <div className="break-keep pb-lg text-center text-body leading-relaxed text-muted-foreground wrap-anywhere">{note}</div> : null}
    </main>
  );
}

export function DeviceIdentity({ name, large = false }: { name: string; large?: boolean }) {
  return (
    <div className={`flex min-w-0 max-w-full items-center gap-sm ${large ? "flex-col gap-lg" : "text-subhead text-subtle-foreground"}`}>
      <LaptopIcon aria-hidden="true" className={large ? "size-(--size-mobile-app-icon) text-muted-foreground" : "size-(--size-icon-lg) shrink-0"} />
      {large ? <h1 className="max-w-full whitespace-normal break-keep text-headline font-semibold wrap-anywhere">{name}</h1> : <span className="min-w-0 whitespace-normal break-keep wrap-anywhere">{name}</span>}
    </div>
  );
}
