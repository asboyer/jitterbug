import { mount } from 'svelte';
import App from './App.svelte';
import MenuBar from './components/MenuBar.svelte';
import ActionPopup from './components/ActionPopup.svelte';
import './app.css';

const view = new URLSearchParams(window.location.search).get('view');
document.documentElement.classList.toggle('menubar-window', view === 'menubar');
document.documentElement.classList.toggle('popup-window', view === 'popup');
mount(view === 'menubar' ? MenuBar : view === 'popup' ? ActionPopup : App, {
    target: document.getElementById('app')!,
});
