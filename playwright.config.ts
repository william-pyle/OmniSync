import {defineConfig} from '@playwright/test';
export default defineConfig({testDir:'tests/ui',fullyParallel:true,workers:2,reporter:'list',use:{channel:'msedge',baseURL:'http://127.0.0.1:1421',viewport:{width:1280,height:900},headless:true},webServer:{command:'npm run dev -- --port 1421 --strictPort',url:'http://127.0.0.1:1421',reuseExistingServer:false},outputDir:'test-results'});
