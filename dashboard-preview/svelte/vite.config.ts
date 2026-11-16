import {sveltekit} from '@sveltejs/kit/vite';
import {defineConfig} from 'vite';
export default defineConfig({plugins:[sveltekit()],server:{proxy:{'/api':{target:'http://192.168.0.14:8765',changeOrigin:true,rewrite:path=>path.slice(4)}}}});
