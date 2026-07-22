const scenes = {
  desktop: {
    kicker: "CORE LOOP / 核心闭环",
    title: "桌面上，只留下一件正在做的事",
    principle: "悬浮层只负责“记下来”和“做下一件”，不把完整管理后台塞进桌面。",
    note: "完成后，下一件事自然顶上来；金币反馈短暂出现，不打断当前工作。",
  },
  capture: {
    kicker: "CAPTURE / 随手记录",
    title: "想到就记，不要求先整理",
    principle: "标题是唯一必填项。新任务安静地进入队尾，不抢走正在处理的事情。",
    note: "全局快捷键、输入、回车，三步完成；日期、标签和项目都留到以后再补。",
  },
  exception: {
    kicker: "REALITY / 真实处理",
    title: "做不了，也要有一个清楚的去处",
    principle: "延后、阻塞和放弃都是正式结果。保留原因，但不扣金币，也不责备用户。",
    note: "任务离开当前队列后自动切换下一件；原因会在本周回顾中重新出现。",
  },
  home: {
    kicker: "REWARD / 正向反馈",
    title: "现实里的完成，变成伙伴的成长",
    principle: "宠物不会死亡、退化或因断签受罚。成长只累积，不制造额外焦虑。",
    note: "金币由确定规则发放，喂食留下账本；宠物只表达结果，不决定奖励。",
  },
  history: {
    kicker: "MEMORY / 完成历史",
    title: "不靠回忆，也知道这一周做过什么",
    principle: "历史保存过程而不只是结果：完成、阻塞、延后和计划调整都能进入回顾。",
    note: "第一版先按规则生成可复制周报；未来 AI 只负责润色，不能改写真实记录。",
  },
};

const canvas = document.querySelector("#desktopCanvas");
const sceneButtons = [...document.querySelectorAll("[data-scene]")];
const panels = [...document.querySelectorAll("[data-scene-panel]")];
const title = document.querySelector("#sceneTitle");
const kicker = document.querySelector("#sceneKicker");
const principleText = document.querySelector("#principleText");
const principleIndex = document.querySelector("#principleIndex");
const sceneNote = document.querySelector("#sceneNote");
const toast = document.querySelector("#demoToast");
const toastText = document.querySelector("#toastText");
const captureInput = document.querySelector("#captureInput");
const captureResult = document.querySelector("#captureResult");
const reasonPanel = document.querySelector("#reasonPanel");
const reasonLabel = document.querySelector("#reasonLabel");
const reasonInput = document.querySelector("#reasonInput");
const currentTaskTitle = document.querySelector("#currentTaskTitle");
const doneCount = document.querySelector("#doneCount");
const completeRing = document.querySelector("#completeTask");
const taskQueue = document.querySelector("#taskQueue");
const growthBar = document.querySelector("#growthBar");
const petSpeech = document.querySelector("#petSpeech");
const initialQueue = taskQueue.innerHTML;

let activeScene = "desktop";
let coinBalance = 12;
let growth = 3;
let taskCompleted = false;
let toastTimer;

function showScene(sceneName) {
  const scene = scenes[sceneName];
  if (!scene) return;

  activeScene = sceneName;
  canvas.dataset.activeScene = sceneName;
  panels.forEach((panel) => panel.classList.toggle("is-visible", panel.dataset.scenePanel === sceneName));
  sceneButtons.forEach((button) => button.classList.toggle("is-active", button.dataset.scene === sceneName));

  const index = Object.keys(scenes).indexOf(sceneName) + 1;
  kicker.textContent = scene.kicker;
  title.textContent = scene.title;
  principleIndex.textContent = `原则 ${String(index).padStart(2, "0")}`;
  principleText.textContent = scene.principle;
  sceneNote.textContent = scene.note;

  if (sceneName === "capture") {
    window.setTimeout(() => captureInput.focus(), 320);
  }
}

function showToast(message) {
  window.clearTimeout(toastTimer);
  toastText.textContent = message;
  toast.classList.add("is-visible");
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 2300);
}

function updateCoins() {
  document.querySelectorAll("[data-coin-count]").forEach((node) => {
    node.textContent = String(coinBalance);
  });
}

function celebratePets() {
  document.querySelectorAll("[data-pet]").forEach((pet) => {
    pet.classList.remove("pet-idle", "is-celebrating");
    void pet.offsetWidth;
    pet.classList.add("is-celebrating");
    window.setTimeout(() => {
      pet.classList.remove("is-celebrating");
      pet.classList.add("pet-idle");
    }, 720);
  });
}

function completeCurrentTask() {
  if (taskCompleted) {
    showToast("这一项已经记入完成历史");
    return;
  }

  taskCompleted = true;
  coinBalance += 1;
  doneCount.textContent = String(Number(doneCount.textContent) + 1);
  updateCoins();
  completeRing.classList.add("is-done");
  celebratePets();
  showToast("任务完成，金币 +1");

  window.setTimeout(() => {
    const firstTask = taskQueue.querySelector("li");
    if (!firstTask) return;
    currentTaskTitle.textContent = firstTask.querySelector("p").textContent;
    firstTask.remove();
    [...taskQueue.children].forEach((item, index) => {
      item.querySelector("span").textContent = String(index + 1);
      item.classList.add("is-shifting");
      window.setTimeout(() => item.classList.remove("is-shifting"), 550);
    });
    completeRing.classList.remove("is-done");
    taskCompleted = false;
  }, 900);
}

function saveCapture() {
  const value = captureInput.value.trim();
  if (!value) {
    showToast("先写下一件要做的事");
    captureInput.focus();
    return;
  }

  captureResult.classList.add("is-shown");
  window.setTimeout(() => captureResult.classList.remove("is-shown"), 1800);
}

function chooseDecision(button) {
  document.querySelectorAll("[data-decision]").forEach((item) => item.classList.remove("is-selected"));
  button.classList.add("is-selected");
  const decision = button.dataset.decision;
  const content = {
    delay: ["再次出现时间", "明天上午 09:00"],
    blocked: ["阻塞原因", "等待测试环境权限"],
    abandon: ["调整原因", "方案已经改变，不再需要"],
  }[decision];
  reasonLabel.textContent = content[0];
  reasonInput.value = content[1];
  reasonPanel.classList.add("is-shown");
  window.setTimeout(() => reasonInput.focus(), 100);
}

function feedPet(button) {
  const cost = Number(button.dataset.cost);
  const food = button.dataset.food;
  if (coinBalance < cost) {
    showToast("金币还不够，先去完成一件事吧");
    return;
  }

  coinBalance -= cost;
  growth = Math.min(5, growth + 1);
  updateCoins();
  growthBar.style.width = `${growth * 20}%`;
  petSpeech.textContent = `${food}真好吃，谢谢你。`;
  celebratePets();
  showToast(`喂食成功，成长 +1`);
}

function resetDemo() {
  coinBalance = 12;
  growth = 3;
  taskCompleted = false;
  doneCount.textContent = "4";
  currentTaskTitle.textContent = "整理桌面待办设计方案";
  taskQueue.innerHTML = initialQueue;
  completeRing.classList.remove("is-done");
  captureInput.value = "更新部署说明文档";
  captureResult.classList.remove("is-shown");
  reasonPanel.classList.remove("is-shown");
  document.querySelectorAll("[data-decision]").forEach((item) => item.classList.remove("is-selected"));
  growthBar.style.width = "60%";
  petSpeech.textContent = "再完成一件，就够买小鱼干啦。";
  updateCoins();
  showScene("desktop");
  showToast("演示已经重置");
}

sceneButtons.forEach((button) => button.addEventListener("click", () => showScene(button.dataset.scene)));
document.querySelectorAll("[data-go-scene]").forEach((button) => {
  button.addEventListener("click", () => showScene(button.dataset.goScene));
});

document.querySelector("#completeTask").addEventListener("click", completeCurrentTask);
document.querySelector("#completeTaskText").addEventListener("click", completeCurrentTask);
document.querySelector("#saveCapture").addEventListener("click", saveCapture);
document.querySelectorAll("[data-decision]").forEach((button) => {
  button.addEventListener("click", () => chooseDecision(button));
});
document.querySelector("#confirmDecision").addEventListener("click", () => {
  const selected = document.querySelector("[data-decision].is-selected");
  if (!selected) {
    showToast("先选择一种处理方式");
    return;
  }
  const labels = { delay: "任务已延后", blocked: "任务已移入阻塞事项", abandon: "任务已保留为计划调整" };
  showToast(`${labels[selected.dataset.decision]}，继续下一件`);
  window.setTimeout(() => showScene("desktop"), 450);
});
document.querySelectorAll(".food-item").forEach((button) => button.addEventListener("click", () => feedPet(button)));
document.querySelector("#copyReport").addEventListener("click", async () => {
  const report = "本周完成：月度巡检报告生成与验证、登录接口异常修复、部署说明文档更新。\n进行中：桌面宠物待办设计。\n阻塞事项：登录功能测试，等待测试环境权限。";
  showToast("正在准备周报文本…");

  let copied = false;
  try {
    if (navigator.clipboard?.writeText) {
      await Promise.race([
        navigator.clipboard.writeText(report),
        new Promise((_, reject) => window.setTimeout(() => reject(new Error("clipboard timeout")), 500)),
      ]);
      copied = true;
    }
  } catch {
    const textarea = document.createElement("textarea");
    textarea.value = report;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    copied = document.execCommand("copy");
    textarea.remove();
  }

  showToast(copied ? "周报已复制，可以继续修改" : "周报草稿已经准备好");
});
document.querySelector("#resetDemo").addEventListener("click", resetDemo);

captureInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") saveCapture();
});

document.addEventListener("keydown", (event) => {
  if (event.ctrlKey && event.code === "Space") {
    event.preventDefault();
    showScene("capture");
  }
  if (event.key === "Escape" && activeScene !== "desktop") {
    showScene("desktop");
  }
});
