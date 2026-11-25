import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // 静的エクスポート(next/image の最適化なし)のため <img> を使う(docs/02-architecture.md §3.1)
  { rules: { "@next/next/no-img-element": "off" } },
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
    "src-tauri/**",
    // ローカル ComfyUI の生成物(venv 内の JS などを走査しない。docs/08)
    "comfyui/**",
  ]),
]);

export default eslintConfig;
