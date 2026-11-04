import { mount } from 'svelte';
import { invoke, isTauri } from '@tauri-apps/api/core';
import App from './App.svelte';
import MenuBar from './components/MenuBar.svelte';
import ActionPopup from './components/ActionPopup.svelte';
import './app.css';
import './product.css';

// The app window cannot open new browser windows itself; hand web links to the default browser.
if (isTauri())
    document.addEventListener('click', (event) => {
        const link = (event.target as Element | null)?.closest?.('a[href^="https://"]');
        if (!(link instanceof HTMLAnchorElement)) return;
        event.preventDefault();
        void invoke('open_link', { url: link.href }).catch(() => {});
    });

const view = new URLSearchParams(window.location.search).get('view');
document.documentElement.classList.toggle('menubar-window', view === 'menubar');
document.documentElement.classList.toggle('popup-window', view === 'popup');
const root = view === 'menubar' ? MenuBar : view === 'popup' ? ActionPopup : App;
mount(root, {
    target: document.getElementById('app')!,
});
