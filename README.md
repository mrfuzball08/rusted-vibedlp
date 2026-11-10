# VibeDLP

Aplicación de escritorio en Rust y Slint para descargar video, audio y listas de reproducción con yt-dlp. La interfaz está en español y admite Linux, macOS y Windows.

## Requisitos

- Rust 1.92 o posterior para compilar. En Windows usa la cadena de herramientas de Rust para MSVC con las herramientas de compilación de Visual Studio. En macOS, instala las herramientas de línea de comandos de Xcode.
- En Linux necesitas un enlazador de C para compilar. La interfaz usa la biblioteca Fontconfig del sistema en tiempo de ejecución. El selector de carpetas usa XDG Desktop Portal con un backend de escritorio o Zenity.
- Instala una versión actual de [yt-dlp](https://github.com/yt-dlp/yt-dlp/wiki/Installation) y coloca el ejecutable en el `PATH` o junto al binario de VibeDLP.
- Instala [FFmpeg y FFprobe](https://ffmpeg.org/download.html) en el mismo directorio, en el `PATH` o junto al binario de VibeDLP. FFmpeg es necesario para extraer audio, elegir el contenedor de video y combinar pistas. Si falta, el modo de video automático busca un formato que ya incluya audio.
- Para enlaces de YouTube, instala los componentes EJS y un entorno JavaScript compatible. VibeDLP admite el Deno que usa yt-dlp por defecto y habilita Node.js si lo detecta. Consulta las versiones y opciones de instalación en la [guía de yt-dlp para YouTube](https://github.com/yt-dlp/yt-dlp/wiki/EJS).

## Ejecutar

```sh
cargo run --locked
```

Para generar un binario optimizado en el sistema donde lo vas a usar, ejecuta `cargo build --release --locked`. El binario queda en `target/release/vibedlp` en Linux y macOS, o `target\release\vibedlp.exe` en Windows.

## Uso

1. Pega el enlace y elige **Video** o **Audio** en **Tipo de descarga**. En **Formato y calidad**, selecciona el perfil de salida. Las resoluciones de video limitan la altura de las pistas disponibles. Las tasas de MP3 se aplican cuando yt-dlp convierte el audio.
2. Activa **Descargar lista completa** para procesar todos los elementos con el formato elegido. Si pegas un enlace directo a una lista con la casilla desactivada, se procesa solo el primer elemento.
3. Pulsa **Descargar**. Durante la transferencia verás el progreso del archivo en curso. Al terminar puedes abrir la carpeta de destino.

El icono de la esquina superior derecha abre **Configuración**, donde puedes seleccionar otra carpeta o restaurar la carpeta de descargas del sistema. También puedes cambiar entre Ayu Dark · Gris, Catppuccin Mocha, Nord y Dracula. La carpeta y el tema se conservan al cerrar la aplicación.
