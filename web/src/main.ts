// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/tokens.css";

const target = document.getElementById("app");
if (!target) throw new Error("#app 요소가 없다");

mount(App, { target });
