import { Toaster } from "sonner";

export function AppToaster() {
  return <Toaster theme="dark" position="top-right" closeButton duration={4500} icons={{
    success: <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} className="h-5 w-5 text-emerald-400"><circle cx={12} cy={12} r={9} /><path d="m8 12 3 3 5-6" strokeLinecap="round" strokeLinejoin="round" /></svg>,
  }} toastOptions={{
    classNames: {
      toast: "border-zinc-700! bg-[#151518]! shadow-xl!",
      title: "text-sm font-medium",
      description: "text-zinc-400!",
      success: "border-emerald-700! text-emerald-300!",
      closeButton: "cursor-pointer! border-zinc-700! bg-zinc-900! text-zinc-300! hover:bg-zinc-800!",
    },
  }} />;
}
