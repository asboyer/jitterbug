import { mount } from 'svelte';
import App from './App.svelte';
import MenuBar from './components/MenuBar.svelte';
import './app.css';

const quickView = new URLSearchParams(window.location.search).get('view') === 'menubar';
document.documentElement.classList.toggle('menubar-window', quickView);
mount(quickView ? MenuBar : App, { target: document.getElementById('app')! });
