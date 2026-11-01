// Lint for the kith page. Besides the usual rules it enforces the spec's palette ban
// (§1.1): only semantic token classes, never `text-blue-500`, `bg-gray-*`, arbitrary
// colours like `bg-[#fff]`, or hex and rgb() literals in components.

import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import ts from 'typescript-eslint';
import svelteConfig from './svelte.config.js';

const PALETTE =
  'slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|black|white';
const UTILITIES =
  'text|bg|border|border-[trblxy]|ring|ring-offset|fill|stroke|from|via|to|outline|decoration|divide|placeholder|caret|accent|shadow';
const RAW_PALETTE = `(^|\\s|:)(${UTILITIES})-(${PALETTE})(-\\d{2,3})?(\\/\\d+)?(\\s|$)`;
const ARBITRARY_COLOR = `(^|\\s|:)(${UTILITIES})-\\[(#|rgb|hsl|oklch)`;
const HEX = '#[0-9a-fA-F]{3}([0-9a-fA-F]{3}([0-9a-fA-F]{2})?)?$';
const message =
  'Use a semantic token (bg-surface, text-ink-2, ...) from tokens.css; raw palette classes and colour literals are banned (spec §1.1).';

const banned = [
  { selector: `Literal[value=/${RAW_PALETTE}/]`, message },
  { selector: `TemplateElement[value.raw=/${RAW_PALETTE}/]`, message },
  { selector: `SvelteLiteral[value=/${RAW_PALETTE}/]`, message },
  { selector: `Literal[value=/${ARBITRARY_COLOR}/]`, message },
  { selector: `SvelteLiteral[value=/${ARBITRARY_COLOR}/]`, message },
  { selector: `Literal[value=/^${HEX}/]`, message },
  { selector: `Literal[value=/^(rgb|hsl)a?\\(/]`, message },
];

export default ts.config(
  {
    ignores: [
      'dist/',
      'node_modules/',
      'src/lib/types.gen.ts',
      'scripts/',
      'playwright-report/',
      'test-results/',
    ],
  },
  js.configs.recommended,
  ...ts.configs.strict,
  ...svelte.configs.recommended,
  {
    languageOptions: { globals: { ...globals.browser } },
  },
  {
    files: ['**/*.svelte', '**/*.svelte.ts', '**/*.svelte.js'],
    languageOptions: {
      parserOptions: {
        projectService: true,
        extraFileExtensions: ['.svelte'],
        parser: ts.parser,
        svelteConfig,
      },
    },
  },
  {
    files: ['src/**/*.ts', 'src/**/*.svelte'],
    ignores: ['src/**/*.test.ts'],
    rules: {
      'no-restricted-syntax': ['error', ...banned],
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
    },
  },
  {
    files: ['*.config.js', '*.config.ts'],
    languageOptions: { globals: { ...globals.node } },
  },
);
