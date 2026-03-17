// @ts-check
import { defineConfig } from 'astro/config';
import { ValidateEnv } from '@julr/vite-plugin-validate-env'
import { z } from 'zod';
import tailwindcss from '@tailwindcss/vite';

// https://astro.build/config
export default defineConfig({
  site: 'https://reviu.dev',
  vite: {
    plugins: [
      tailwindcss(), 
      ValidateEnv({
        validator: 'standard',
        schema: {
          PUBLIC_BACKEND_URL: z.string(),
        },
      }),
    ],
  }
});
