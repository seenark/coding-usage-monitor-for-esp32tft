import { defineConfig } from "vite-plus";

export default defineConfig({
  lint: {
    ignorePatterns: [
      "node_modules/**",
      "**/node_modules/**",
      "dist/**",
      "apps/web/.tanstack/**",
      "apps/web/src/routeTree.gen.ts",
    ],
    options: {
      typeAware: false,
      typeCheck: false,
    },
  },
  fmt: {
    ignorePatterns: [
      "node_modules/**",
      "**/node_modules/**",
      "dist/**",
      "apps/web/.tanstack/**",
      "apps/web/src/routeTree.gen.ts",
      "packages/ui/**",
    ],
    singleQuote: false,
    semi: true,
    sortPackageJson: true,
  },
});
