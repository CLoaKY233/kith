import { Toast, ToastClose, ToastDescription, ToastRegion, ToastTitle } from "@ui-sdk/react/toast";

export { toast } from "@ui-sdk/react/toast";

export function Toaster() {
  return (
    <ToastRegion className="fixed !m-0 right-4 left-4 bottom-[max(16px,env(safe-area-inset-bottom))] z-50 flex w-auto max-w-none flex-col gap-2 sm:left-auto sm:w-[360px]">
      {() => (
        <Toast className="flex items-center gap-3 text-[14px] text-fg">
          <div className="min-w-0 flex-1">
            <ToastTitle className="block font-normal [overflow-wrap:anywhere] in-data-[type=error]:text-hot" />
            <ToastDescription className="mt-0.5 block text-[13px] text-fg-2 [overflow-wrap:anywhere]" />
          </div>
          <ToastClose aria-label="Dismiss" className="shrink-0 text-fg-3 hover:text-fg" />
        </Toast>
      )}
    </ToastRegion>
  );
}
