import { Suspense } from "react";

import { GalleryView } from "@/components/gallery/gallery-view";

export default function GalleryPage() {
  // useSearchParams(?id=)を使うため、静的エクスポートでは Suspense 境界が必要
  return (
    <Suspense>
      <GalleryView />
    </Suspense>
  );
}
