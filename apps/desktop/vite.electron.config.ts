import { builtinModules } from "node:module";
import { resolve } from "node:path";
import { defineConfig } from "vite";

const external = [
  "electron",
  ...builtinModules,
  ...builtinModules.map((name) => `node:${name}`),
];

export default defineConfig(({ mode }) => {
  if (mode !== "main" && mode !== "preload") {
    throw new Error("Electron build mode must be 'main' or 'preload'");
  }

  return {
    build: {
      emptyOutDir: true,
      lib: {
        entry: resolve(__dirname, `src/${mode}/index.ts`),
        formats: ["cjs"],
      },
      outDir: `dist/${mode}`,
      rollupOptions: {
        external,
        output: {
          entryFileNames: "index.cjs",
          inlineDynamicImports: true,
        },
      },
      target: "node22",
    },
  };
});
