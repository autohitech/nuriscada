// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import type { AlarmRecord, DeviceStatus, StatusDoc, StreamMessage, TagSnapshot } from "./types";

/** 화면에 보관하는 최근 알람 기록 수. */
export const RECENT_LIMIT = 200;

/**
 * 서버 상태의 화면용 사본. 태그는 이름 순서가 바뀌지 않는 배열로 두고, 한 태그가 바뀌면 그 칸만
 * 다시 쓴다(세밀한 반응성). 그래서 값이 바뀐 행만 다시 그려진다.
 */
export class Store {
  tags = $state<TagSnapshot[]>([]);
  /** 태그 이름 목록(검색용). 값이 바뀌어도 바뀌지 않는다. */
  names = $state<string[]>([]);
  /** 알람 상태가 정상이 아닌 태그의 이름. */
  activeNames = $state<string[]>([]);
  recent = $state<AlarmRecord[]>([]);
  devices = $state<DeviceStatus[]>([]);
  connected = $state(false);
  /** 마지막 알람 기록을 낭독용 문구로 바꾸기 위한 최신 기록. */
  latestAlarm = $state<AlarmRecord | null>(null);

  #index = new Map<string, number>();

  /** 스트림 메시지 하나를 반영한다. */
  apply(msg: StreamMessage): void {
    switch (msg.type) {
      case "snapshot": {
        this.tags = msg.data.tags;
        this.names = msg.data.tags.map((t) => t.name);
        this.#index = new Map(this.names.map((n, i) => [n, i]));
        this.activeNames = msg.data.active.map((t) => t.name);
        this.recent = msg.data.recent.slice(-RECENT_LIMIT);
        this.latestAlarm = null;
        break;
      }
      case "tag": {
        const i = this.#index.get(msg.data.name);
        if (i === undefined) return;
        this.tags[i] = msg.data;
        const isActive = msg.data.alarm_state !== null && msg.data.alarm_state !== "NORMAL";
        const was = this.activeNames.includes(msg.data.name);
        if (isActive && !was) this.activeNames.push(msg.data.name);
        if (!isActive && was) this.activeNames = this.activeNames.filter((n) => n !== msg.data.name);
        break;
      }
      case "alarm": {
        this.recent.push(msg.data);
        if (this.recent.length > RECENT_LIMIT) this.recent.splice(0, this.recent.length - RECENT_LIMIT);
        this.latestAlarm = msg.data;
        break;
      }
    }
  }

  setStatus(doc: StatusDoc): void {
    this.devices = doc.devices;
  }

  /** 이름으로 태그 위치를 찾는다. */
  indexOf(name: string): number | undefined {
    return this.#index.get(name);
  }
}

/** 웹소켓 팩토리(시험에서 바꿔 끼운다). */
export type SocketFactory = (url: string) => WebSocket;

export interface ConnectOptions {
  url: string;
  store: Store;
  socket?: SocketFactory;
  /** 재연결 대기 시간(밀리초)의 시작과 상한. */
  backoff?: { initial: number; max: number };
}

/** 스트림에 연결한다. 끊기면 지수적으로 늘리며 다시 연결한다. 반환 함수를 부르면 멈춘다. */
export function connect({ url, store, socket = (u) => new WebSocket(u), backoff = { initial: 500, max: 5000 } }: ConnectOptions): () => void {
  let stopped = false;
  let ws: WebSocket | undefined;
  let delay = backoff.initial;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const open = () => {
    if (stopped) return;
    ws = socket(url);
    ws.onopen = () => {
      store.connected = true;
      delay = backoff.initial;
    };
    ws.onmessage = (ev) => {
      try {
        store.apply(JSON.parse(String(ev.data)) as StreamMessage);
      } catch {
        // 알 수 없는 메시지는 무시한다.
      }
    };
    ws.onclose = () => {
      store.connected = false;
      if (stopped) return;
      timer = setTimeout(open, delay);
      delay = Math.min(delay * 2, backoff.max);
    };
    ws.onerror = () => ws?.close();
  };

  open();
  return () => {
    stopped = true;
    if (timer) clearTimeout(timer);
    ws?.close();
  };
}

/** 장치 상태를 주기적으로 가져온다. 반환 함수를 부르면 멈춘다. */
export function pollStatus(store: Store, fetcher: typeof fetch = fetch, everyMs = 2000): () => void {
  let stopped = false;
  const tick = async () => {
    try {
      const res = await fetcher("/api/v1/status");
      if (res.ok) store.setStatus((await res.json()) as StatusDoc);
    } catch {
      // 서버에 닿지 않으면 다음 주기에 다시 시도한다.
    }
  };
  void tick();
  const id = setInterval(() => {
    if (!stopped) void tick();
  }, everyMs);
  return () => {
    stopped = true;
    clearInterval(id);
  };
}
