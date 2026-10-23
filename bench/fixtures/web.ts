export function render(el: HTMLElement, html: string) {
  el.innerHTML = html;
}

export function save(token: string) {
  localStorage.setItem("auth_token", token);
}

export function rnd() {
  return Math.random();
}
