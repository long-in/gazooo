import path from "node:path";

import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "tools/**/*.test.ts"],
    // Node 26(26.13.1 で確認)では既定のプール(forks/threads)だと外部化された "vitest" が
    // 二重に読み込まれ、全テストが "Vitest failed to find the runner" で落ちるため VM 系を使う。
    // Vitest 更新時に既定プールで通るか再確認し、通れば外す(docs/26 §3)。
    pool: "vmThreads",
  },
});
