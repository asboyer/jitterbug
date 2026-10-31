import { mount } from 'svelte';
import App from './App.svelte';
import MenuBar from './components/MenuBar.svelte';
import ActionPopup from './components/ActionPopup.svelte';
import Lab from './components/Lab.svelte';
import './app.css';

const view = new URLSearchParams(window.location.search).get('view');
document.documentElement.classList.toggle('menubar-window', view === 'menubar');
document.documentElement.classList.toggle('popup-window', view === 'popup');
const root =
    view === 'menubar' ? MenuBar : view === 'popup' ? ActionPopup : view === 'lab' ? Lab : App;
mount(root, {
    target: document.getElementById('app')!,
});
