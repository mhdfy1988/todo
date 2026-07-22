/**
 * 将唯一的宠物模板克隆到四种窗口形态，避免内部结构复制后逐渐漂移。
 * @param {Document} document
 */
export function mountPetComponents(document) {
  const template = document.querySelector("#petTemplate");
  if (!(template instanceof HTMLTemplateElement)) {
    throw new Error("界面缺少宠物模板：#petTemplate");
  }
  document.querySelectorAll("[data-pet]").forEach((host) => {
    if (!host.hasChildNodes()) {
      host.append(template.content.cloneNode(true));
    }
  });
}
