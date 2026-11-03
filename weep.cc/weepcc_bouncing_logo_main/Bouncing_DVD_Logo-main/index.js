let speed = 50;
let scale = 0.25;
let canvas;
let ctx;
let logoColor;
let lasTime = performance.now();

let logo = {
    x: 200,
    y: 300,
    xspeed: 300,
    yspeed: 300,
    img: new Image()
};

(function main() {
    canvas = document.getElementById("WEEPCCLogo");
    ctx = canvas.getContext("2d");
    logo.img.src = 'weepcc_logo_2.png';
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
    chooseColor();
    update();
})();


function update() {
    const now = performance.now();
    // the time is in ms and we want to convert it to seconds
    const delta = (lasTime - now) / 1000;
    // set the lastTime to this frame's render time
    lasTime = now;

    //Draw the canvas background
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    //Draw DVD Logo and his background
    ctx.fillStyle = logoColor;
    ctx.fillRect(logo.x, logo.y, logo.img.width * scale, logo.img.height * scale);
    ctx.drawImage(logo.img, logo.x, logo.y, logo.img.width * scale, logo.img.height * scale);

    // DVD logo movement
    logo.x += logo.xspeed * delta;
    logo.y += logo.yspeed * delta;
    //Check for collision 
    checkColission();

    requestAnimationFrame(update);
}

// collision check
function checkColission() {
    if (logo.x + logo.img.width * scale >= canvas.width || logo.x <= 0) {
        logo.xspeed *= -1;
        chooseColor();
    }

    if (logo.y + logo.img.height * scale >= canvas.height || logo.y <= 0) {
        logo.yspeed *= -1;
        chooseColor();
    }
}

// randomize color in rgb
function chooseColor() {
    r = Math.random() * (254 - 0) + 0;
    g = Math.random() * (254 - 0) + 0;
    b = Math.random() * (254 - 0) + 0;

    logoColor = 'rgb(' + r + ',' + g + ', ' + b + ')';
}