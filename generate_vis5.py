import os

html_content = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
<title>L++ Cubic CFG Matrix Visualization</title>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;800&display=swap" rel="stylesheet">
<style>
  body { 
    margin: 0; 
    overflow: hidden; 
    background-color: #030305; 
    background-image: radial-gradient(circle at 50% 50%, #0a0a12 0%, #000000 100%);
    font-family: 'Inter', sans-serif; 
    color: #e2e8f0; 
    touch-action: none; 
  }
  canvas { 
    position: absolute; 
    top: 0; left: 0; 
    z-index: 1; 
    cursor: grab;
  }
  canvas:active { cursor: grabbing; }
  
  /* Collapsible Glass Panels */
  .glass-panel {
    position: absolute;
    z-index: 10;
    background: rgba(15, 15, 20, 0.4); 
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    border: 1px solid rgba(255, 255, 255, 0.05);
    border-radius: 8px;
    padding: 15px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
    pointer-events: auto;
    transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.3s;
  }
  
  .glass-panel.collapsed { transform: translateY(-100%); opacity: 0; pointer-events: none; }
  
  #ui-container { top: 15px; left: 15px; width: 280px; display: flex; flex-direction: column; gap: 12px; }
  .controls-hint { bottom: 15px; left: 15px; padding: 12px; }
  .controls-hint.collapsed { transform: translateX(-100%); }
  .legend { bottom: 15px; right: 15px; padding: 12px; }
  .legend.collapsed { transform: translateX(100%); }
  
  #toggleUI {
    position: absolute; top: 15px; right: 15px; z-index: 20;
    background: rgba(15, 15, 20, 0.6); border: 1px solid rgba(255, 255, 255, 0.1);
    color: #94a3b8; padding: 8px 12px; border-radius: 6px; cursor: pointer;
    font-size: 12px; font-weight: 600; backdrop-filter: blur(4px); transition: all 0.2s;
  }
  #toggleUI:hover { background: rgba(255, 255, 255, 0.1); color: #fff; }
  
  h2 { margin: 0 0 4px 0; font-size: 15px; font-weight: 800; background: linear-gradient(90deg, #fff, #94a3b8); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
  p.subtitle { margin: 0 0 10px 0; font-size: 11px; color: #94a3b8; line-height: 1.3; }
  
  .control-group { display: flex; flex-direction: column; gap: 6px; }
  .control-row { display: flex; justify-content: space-between; align-items: center; font-size: 11px; font-weight: 600; }
  
  button.action-btn { padding: 8px 16px; background: linear-gradient(135deg, #f97316, #ea580c); color: white; border: none; cursor: pointer; border-radius: 6px; font-weight: 600; font-size: 12px; transition: all 0.2s ease; }
  button.action-btn:hover { background: linear-gradient(135deg, #fb923c, #f97316); box-shadow: 0 0 10px rgba(234, 88, 12, 0.4); }
  
  input[type=range] { -webkit-appearance: none; width: 100%; background: transparent; }
  input[type=range]::-webkit-slider-thumb { -webkit-appearance: none; height: 12px; width: 12px; border-radius: 50%; background: #f97316; cursor: pointer; margin-top: -4px; box-shadow: 0 0 8px rgba(249, 115, 22, 0.5); }
  input[type=range]::-webkit-slider-runnable-track { width: 100%; height: 3px; cursor: pointer; background: rgba(255, 255, 255, 0.1); border-radius: 2px; }
  
  .legend-item { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; font-size: 11px; font-weight: 600; color: #cbd5e1; }
  .legend-item:last-child { margin-bottom: 0; }
  .dot { width: 8px; height: 8px; border-radius: 50%; }
  .keybind { display: inline-block; background: #1e293b; padding: 2px 4px; border-radius: 3px; border: 1px solid #334155; font-family: monospace; font-size: 10px; margin-right: 4px; color: #94a3b8;}
</style>
</head>
<body>

<button id="toggleUI">Hide UI (Press H)</button>

<div id="ui-container" class="glass-panel panel">
  <h2>L++ Static Analyzer</h2>
  <p class="subtitle">3D Cubic CFG Matrix (Simulated)</p>
  
  <button id="playPause" class="action-btn">⏸ Pause</button>
  
  <div class="control-group" style="margin-top: 5px;">
    <div class="control-row">
      <span>Speed</span>
      <span id="speedLabel" style="color: #f97316;">Fast</span>
    </div>
    <input type="range" id="speedSlider" min="1" max="100" value="80">
  </div>
</div>

<div class="controls-hint glass-panel panel">
  <div class="legend-item"><span class="keybind">Left Click</span> Rotate</div>
  <div class="legend-item"><span class="keybind">Right Click</span> Pan</div>
  <div class="legend-item"><span class="keybind">Scroll</span> Zoom</div>
  <div class="legend-item"><span class="keybind">Spacebar</span> Snap to Center</div>
</div>

<div class="legend glass-panel panel">
  <div class="legend-item"><div class="dot" style="background:#f97316; box-shadow: 0 0 8px #f97316;"></div> Path Trace</div>
  <div class="legend-item"><div class="dot" style="background:#ef4444;"></div> Alloc Node</div>
  <div class="legend-item"><div class="dot" style="background:#22c55e;"></div> Free Node</div>
  <div class="legend-item"><div class="dot" style="background:rgba(96, 165, 250, 0.6);"></div> Basic Block</div>
</div>

<canvas id="canvas"></canvas>

<script>
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d', { alpha: false }); 
const playPauseBtn = document.getElementById('playPause');
const speedSlider = document.getElementById('speedSlider');
const speedLabel = document.getElementById('speedLabel');
const toggleUIBtn = document.getElementById('toggleUI');
const panels = document.querySelectorAll('.panel');

let uiVisible = true;

function toggleUI() {
    uiVisible = !uiVisible;
    panels.forEach(p => {
        if(uiVisible) p.classList.remove('collapsed');
        else p.classList.add('collapsed');
    });
    toggleUIBtn.innerText = uiVisible ? "Hide UI (Press H)" : "Show UI (Press H)";
}

toggleUIBtn.onclick = toggleUI;
window.addEventListener('keydown', (e) => {
    if(e.key.toLowerCase() === 'h') toggleUI();
    if(e.key === ' ') snapToCenter(); 
});

let width, height, cx, cy;
function resize() {
    width = canvas.width = window.innerWidth;
    height = canvas.height = window.innerHeight;
    cx = width / 2;
    cy = height / 2;
}
window.addEventListener('resize', resize);
resize();

// UI State
let isPlaying = true;
let stepIndex = 0;

// Camera State (Zoomed out slightly to see the cube)
let rotationX = 0.5;
let rotationY = 0.5;
let autoRotateY = 0.002;
let autoRotateX = 0.0005;
let cameraZ = 2200;      
let panX = 0;            
let panY = 0;            
let isSnapping = false;

function snapToCenter() {
    isSnapping = true;
    panX = 0;
    panY = 0;
    cameraZ = 2200;
}

// Cubic Graph Data Structure
const nodes = [];
const edges = [];

// Create a 12x12x12 Cube Grid (1728 nodes)
const GRID_SIZE = 12;
const SPACING = 70; // Distance between nodes
const OFFSET = (GRID_SIZE * SPACING) / 2;
const numNodes = GRID_SIZE * GRID_SIZE * GRID_SIZE;

function getIndex(x, y, z) {
    return x + y * GRID_SIZE + z * GRID_SIZE * GRID_SIZE;
}

function buildCubicGraph() {
    for(let x = 0; x < GRID_SIZE; x++) {
        for(let y = 0; y < GRID_SIZE; y++) {
            for(let z = 0; z < GRID_SIZE; z++) {
                let px = (x * SPACING) - OFFSET + (SPACING/2);
                let py = (y * SPACING) - OFFSET + (SPACING/2);
                let pz = (z * SPACING) - OFFSET + (SPACING/2);

                let rand = Math.random();
                let type = 'basic';
                if (rand > 0.97) type = 'alloc';
                else if (rand > 0.94) type = 'free';
                else if (rand > 0.8) type = 'branch';

                let id = getIndex(x, y, z);
                nodes[id] = { id: id, x: px, y: py, z: pz, type: type };

                // Build structural edges (connecting adjacent grid nodes)
                // Probability controls how dense the cube's internal webbing is
                if (x > 0 && Math.random() > 0.25) edges.push({ from: getIndex(x-1, y, z), to: id });
                if (y > 0 && Math.random() > 0.25) edges.push({ from: getIndex(x, y-1, z), to: id });
                if (z > 0 && Math.random() > 0.25) edges.push({ from: getIndex(x, y, z-1), to: id });
                
                // Add some cross-cube 'goto' jumps
                if (Math.random() > 0.99 && id > 50) {
                    edges.push({ from: id, to: Math.floor(Math.random() * id) });
                }
            }
        }
    }
}
buildCubicGraph();

// Pre-calculate a random walk traversal through the cube
const traversal = [0];
let curr = 0;
for(let i = 0; i < 6000; i++) {
    let outs = edges.filter(e => e.from === curr || e.to === curr); // bidirectional for the walk
    if (outs.length === 0) curr = 0; 
    else {
        let edge = outs[Math.floor(Math.random() * outs.length)];
        curr = (edge.from === curr) ? edge.to : edge.from;
    }
    traversal.push(curr);
}

const projX = new Float32Array(numNodes);
const projY = new Float32Array(numNodes);
const projScale = new Float32Array(numNodes);

function project() {
    let cosY = Math.cos(rotationY), sinY = Math.sin(rotationY);
    let cosX = Math.cos(rotationX), sinX = Math.sin(rotationX);
    
    for(let i=0; i<numNodes; i++) {
        let n = nodes[i];
        if (!n) continue;
        let nx = n.x * cosY - n.z * sinY;
        let nz = n.z * cosY + n.x * sinY;
        let ny = n.y * cosX - nz * sinX;
        nz = nz * cosX + n.y * sinX;
        
        let scale = cameraZ / (cameraZ + nz);
        
        projX[i] = cx + panX + (nx * scale);
        projY[i] = cy + panY + (ny * scale);
        projScale[i] = scale;
    }
}

let isDraggingLeft = false;
let isDraggingRight = false;
let lastX = 0, lastY = 0;

canvas.addEventListener('contextmenu', e => e.preventDefault());

canvas.addEventListener('mousedown', e => { 
    isSnapping = false; 
    if(e.button === 0) isDraggingLeft = true;
    if(e.button === 2) isDraggingRight = true;
    lastX = e.clientX; lastY = e.clientY; 
});

window.addEventListener('mouseup', e => { 
    if(e.button === 0) isDraggingLeft = false;
    if(e.button === 2) isDraggingRight = false;
});

window.addEventListener('mousemove', e => {
    if (isDraggingLeft) {
        rotationY += (e.clientX - lastX) * 0.005;
        rotationX += (e.clientY - lastY) * 0.005;
    } else if (isDraggingRight) {
        panX += (e.clientX - lastX);
        panY += (e.clientY - lastY);
    }
    if (isDraggingLeft || isDraggingRight) {
        lastX = e.clientX; 
        lastY = e.clientY;
    }
});

canvas.addEventListener('wheel', e => {
    isSnapping = false;
    e.preventDefault();
    cameraZ += e.deltaY * 2.0;
    if (cameraZ < 200) cameraZ = 200;
    if (cameraZ > 8000) cameraZ = 8000;
}, { passive: false });

function render() {
    if (!isDraggingLeft && !isDraggingRight && isPlaying) {
        rotationY += autoRotateY;
        rotationX += autoRotateX;
    }
    
    if (isSnapping) {
        let diff = Math.abs(rotationY - 0.5) + Math.abs(rotationX - 0.5);
        rotationY += (0.5 - rotationY) * 0.1;
        rotationX += (0.5 - rotationX) * 0.1;
        if (diff < 0.01) isSnapping = false;
    }
    
    project();

    ctx.fillStyle = '#050508';
    ctx.fillRect(0, 0, width, height);

    // Draw cube internal webbing/edges
    ctx.beginPath();
    ctx.strokeStyle = 'rgba(71, 85, 105, 0.1)'; 
    ctx.lineWidth = 0.5;
    for(let i=0; i<edges.length; i++) {
        let f = edges[i].from;
        let t = edges[i].to;
        if (projScale[f] > 0 && projScale[t] > 0) {
            ctx.moveTo(projX[f], projY[f]);
            ctx.lineTo(projX[t], projY[t]);
        }
    }
    ctx.stroke();

    // Draw path tail
    ctx.globalCompositeOperation = 'screen';
    let startIdx = Math.max(0, stepIndex - 40);
    for(let i = startIdx; i < stepIndex; i++) {
        let f = traversal[i];
        let t = traversal[i+1];
        if (projScale[f] > 0 && projScale[t] > 0) {
            ctx.beginPath();
            ctx.moveTo(projX[f], projY[f]);
            ctx.lineTo(projX[t], projY[t]);
            let progress = (i - startIdx) / 40; 
            ctx.lineWidth = 3 * progress * ((projScale[f] + projScale[t])/2);
            ctx.strokeStyle = `rgba(249, 115, 22, ${progress})`; 
            ctx.stroke();
        }
    }
    ctx.globalCompositeOperation = 'source-over'; 

    const drawNodesByType = (typeFilter, color, sizeMultiplier) => {
        ctx.fillStyle = color;
        ctx.beginPath();
        for(let i=0; i<numNodes; i++) {
            if (!nodes[i] || projScale[i] <= 0 || nodes[i].type !== typeFilter) continue;
            if (projScale[i] < 0.3 && typeFilter === 'basic') continue; 
            
            let r = Math.max(0.5, sizeMultiplier * projScale[i]);
            ctx.moveTo(projX[i] + r, projY[i]);
            ctx.arc(projX[i], projY[i], r, 0, 6.28318);
        }
        ctx.fill();
    };

    drawNodesByType('basic', 'rgba(96, 165, 250, 0.3)', 1.5);
    drawNodesByType('branch', 'rgba(96, 165, 250, 0.7)', 2);
    drawNodesByType('alloc', '#ef4444', 3);
    drawNodesByType('free', '#22c55e', 3);

    let headId = traversal[stepIndex];
    if (projScale[headId] > 0) {
        ctx.beginPath();
        ctx.fillStyle = '#fff';
        let hr = 5 * projScale[headId];
        ctx.arc(projX[headId], projY[headId], hr, 0, 6.283);
        ctx.fill();
        
        ctx.beginPath();
        ctx.strokeStyle = 'rgba(249, 115, 22, 0.8)';
        ctx.lineWidth = 2 * projScale[headId];
        ctx.arc(projX[headId], projY[headId], hr + 4*projScale[headId], 0, 6.283);
        ctx.stroke();
    }

    requestAnimationFrame(render);
}

function logicTick() {
    if (isPlaying) {
        stepIndex = (stepIndex + 1) % traversal.length;
    }
    let speedVal = parseInt(speedSlider.value);
    let delay = speedVal === 100 ? 5 : (100 - speedVal) * 5; 
    setTimeout(logicTick, delay);
}

playPauseBtn.onclick = () => {
    isPlaying = !isPlaying; 
    playPauseBtn.innerText = isPlaying ? '⏸ Pause' : '▶ Resume';
};

speedSlider.oninput = () => {
    const v = parseInt(speedSlider.value);
    if (v > 80) speedLabel.innerText = 'Fast';
    else if (v < 20) speedLabel.innerText = 'Slow';
    else speedLabel.innerText = 'Normal';
};

render();
logicTick();
</script>
</body>
</html>
"""

escaped_html = html_content.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')

lpp_code = f'''# packages/lpp-analyzer/src/visualization.lpp
# Generates an interactive N-Dimensional visualization of the analyzer's Ownership and CFG graphs.

def generate_html_visualization(output_path: Str) -> Int:
    mut html := "{escaped_html}"
    return write_file(output_path, html)
'''

with open('packages/lpp-analyzer/src/visualization.lpp', 'w') as f:
    f.write(lpp_code)

print("Generated Cubic UI visualization.lpp successfully.")
