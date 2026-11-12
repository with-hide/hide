import { Toaster as Sonner, type ToasterProps } from "sonner";

// Transient confirmations (a copy landed, a background step finished). A state
// the operator must act on stays in its own row, never only in a toast
// (design 13).
function Toaster({ theme, ...props }: ToasterProps) {
  return (
    <Sonner
      theme={theme}
      className="toaster group"
      toastOptions={{
        classNames: {
          toast: "!rounded-md !border !border-border !bg-popover !text-popover-foreground !text-body !shadow-overlay",
          description: "!text-subtle-foreground",
          actionButton: "!bg-primary !text-primary-foreground",
          cancelButton: "!bg-secondary !text-secondary-foreground",
        },
      }}
      {...props}
    />
  );
}

export { Toaster };
