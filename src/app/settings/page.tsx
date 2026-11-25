import { Suspense } from "react";

import { SettingsForm } from "@/components/settings/settings-form";

export default function SettingsPage() {
  // useSearchParams(?tab=)を使うため、静的エクスポートでは Suspense 境界が必要
  return (
    <Suspense>
      <SettingsForm />
    </Suspense>
  );
}
