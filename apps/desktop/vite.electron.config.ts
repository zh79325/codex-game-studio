import { builtinModules } from "node:module";
import { resolve } from "node:path";
import { defineConfig } from "vite";

const external = ["electron", ...builtinModules, ...builtinModules.map((name) => `node:${name}`)];

export default defineConfig({
  build: {
    emptyOutDir: false,
    lib: {
      entry: {
        "main/index": resolve(__dirname, "src/main/index.ts"),
        "preload/index": resolve(__dirname, "src/preload/index.ts"),
      },
      formats: ["cjs"],
    },
    outDir: "dist",
    rollupOptions: {
      external,
      output: {
        entryFileNames: "[name].cjs",
      },
    },
    target: "node22",
  },
});
