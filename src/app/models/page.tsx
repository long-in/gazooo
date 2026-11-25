import { Suspense } from "react";

import { ModelsView } from "@/components/models/models-view";

export default function ModelsPage() {
  // useSearchParams(?category=&base=)を使うため、静的エクスポートでは Suspense 境界が必要
  return (
    <Suspense>
      <ModelsView />
    </Suspense>
  );
}
