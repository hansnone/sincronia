# Sincronia — Guía Completa

Motor de respaldo NAS de alto rendimiento para Windows 11, optimizado para redes 10 GbE.

---

## Tabla de Contenidos

1. [Requisitos](#requisitos)
2. [Ejecución con Cargo (desarrollo)](#ejecución-con-cargo)
3. [Compilación y obtención del EXE](#compilación-del-exe)
4. [Configuración](#configuración)
5. [Tarea Programada de Windows](#tarea-programada)
6. [Despliegue en un equipo de producción](#despliegue)
7. [Arquitectura del sistema](#arquitectura)
8. [Referencia de módulos](#referencia-de-módulos)
9. [Solución de problemas](#solución-de-problemas)

---

## Requisitos

- **Sistema operativo**: Windows 10/11 (x64)
- **Rust toolchain**: `stable-x86_64-pc-windows-msvc` (1.85+)
- **Red**: acceso al NAS vía SMB (\\\\servidor\\recurso)
- **RAM**: ~128 MiB base + (workers × buffer). Con 8 workers × 16 MiB = ~256 MiB total
- **NO requiere privilegios de administrador** (salvo crear la tarea programada)

---

## Ejecución con Cargo

### Primer paso: generar la configuración

```powershell
cd C:\Users\thebe\Desktop\Proyectos\sincronia
cargo run -- --generate-config sincronia.toml
```

Esto crea `sincronia.toml` con todos los parámetros documentados. **Edítalo** antes de ejecutar (al menos `source_directory_path` y las rutas UNC del NAS).

### Ejecutar en modo normal (con icono en bandeja)

```powershell
cargo run -- --config sincronia.toml
```

Aparecerá un icono en la bandeja del sistema (círculo verde/amarillo/rojo según el estado). Clic derecho para ver el menú: pausar, reanudar, abrir logs, etc.

### Ejecutar sin bandeja (modo depuración con logs en consola)

```powershell
cargo run -- --config sincronia.toml --no-tray
```

Útil para ver la salida de tracing en tiempo real durante el desarrollo.

### Opciones disponibles

| Opción | Descripción |
|---|---|
| `--config <ruta>` | Ruta al archivo TOML (default: `sincronia.toml` junto al .exe) |
| `--generate-config [ruta]` | Genera un archivo de configuración de ejemplo |
| `--create-scheduled-task` | Crea la tarea programada de Windows |
| `--no-tray` | Ejecutar sin icono en bandeja (útil para depuración) |
| `--version`, `-v` | Muestra la versión |
| `--help`, `-h` | Muestra la ayuda |

---

## Compilación del EXE

### Build de release (optimizado)

```powershell
cd C:\Users\thebe\Desktop\Proyectos\sincronia
cargo build --release
```

El ejecutable se genera en:

```
target\release\sincronia.exe
```

### Características del build release

El perfil de release en `Cargo.toml` está configurado para máximo rendimiento:

- `opt-level = 3` — optimización máxima
- `lto = "thin"` — Link-Time Optimization
- `codegen-units = 1` — mejor optimización a costa de tiempo de compilación
- `strip = "symbols"` — elimina símbolos de debug (EXE más pequeño)

### Verificar que compila correctamente

```powershell
cargo check          # Solo verificar sin generar binario (rápido)
cargo build --release  # Compilar release completo
```

---

## Configuración

El archivo `sincronia.toml` controla todo el comportamiento. Secciones principales:

### [general]
- `run_mode`: `"backup_append_only"` (respaldar sin borrar) o `"move_after_verified_copy"` (mover tras verificar hash)

### [source]
- `source_directory_path`: carpeta a monitorizar (ej: `"C:\\ColaEntrada"`)
- `minimum_file_stable_seconds`: segundos sin cambios para considerar un archivo estable (default: 60)

### [nas]
- `required_drive_letter`: letra de unidad (ej: `"R:"`)
- `primary_unc_path`: ruta UNC del NAS (ej: `"\\\\RAW-NAS\\Repositorio"`)
- `maximum_credential_prompt_attempts`: intentos del diálogo de credenciales (default: 3)

### [copy_engine]
- `worker_count`: hilos de copia concurrentes (default: 8)
- `copy_buffer_size_mib_per_worker`: buffer por worker en MiB (default: 16)

### [verification]
- `hash_algorithm`: `"blake3"` (rápido, SIMD) o `"sha256"`
- `verification_mode`: `"full_hash"` o `"none"`

### [logging]
- Tres archivos de log: `.log` (humano), `.csv` (métricas), `.jsonl` (eventos)
- Se crean automáticamente los directorios si no existen

### [scheduled_task]
- `delay_after_logon_seconds`: espera tras login antes de arrancar (default: 30)
- `run_only_when_user_is_logged_on`: **debe ser `true`** para que el diálogo de credenciales funcione

> **Consulta el archivo `sincronia.example.toml` para la referencia completa** con todos los parámetros y sus comentarios.

---

## Tarea Programada

### Crear la tarea (una sola vez)

Asegúrate de que la sección `[scheduled_task]` del TOML está configurada, y ejecuta:

```powershell
# Desde el directorio del proyecto (desarrollo)
cargo run -- --config sincronia.toml --create-scheduled-task

# O con el EXE compilado
sincronia.exe --config C:\Sincronia\sincronia.toml --create-scheduled-task
```

Esto crea una tarea en el **Programador de tareas de Windows** que:

1. Se dispara al iniciar sesión el usuario actual
2. Espera 30 segundos (configurable) para que el sistema arranque
3. Ejecuta `sincronia.exe --config <ruta>`
4. Impide instancias paralelas
5. Se reinicia automáticamente hasta 3 veces si falla (cada 5 minutos)
6. No se detiene al cambiar a batería

### Verificar la tarea

```powershell
# Ver la tarea en PowerShell
Get-ScheduledTask -TaskName "Sincronia NAS Backup"

# O abrir el Programador de tareas gráficamente
taskschd.msc
```

### Eliminar la tarea

```powershell
Unregister-ScheduledTask -TaskName "Sincronia NAS Backup" -Confirm:$false
```

### Configuración importante para la tarea

| Parámetro | Valor recomendado | Por qué |
|---|---|---|
| `run_only_when_user_is_logged_on` | `true` | **Obligatorio**: el diálogo de credenciales necesita una sesión interactiva |
| `run_with_highest_privileges` | `false` | No necesario para operación normal |
| `delay_after_logon_seconds` | `30` | Da tiempo a que la red esté disponible |

---

## Despliegue

### Pasos para desplegar en un equipo de producción

```
1. Compilar en tu máquina de desarrollo
   cargo build --release

2. Copiar al equipo destino:
   - sincronia.exe          → C:\Sincronia\sincronia.exe
   - sincronia.toml         → C:\Sincronia\sincronia.toml

3. Editar sincronia.toml en el equipo destino:
   - source_directory_path  → carpeta real a monitorizar
   - primary_unc_path       → \\servidor\recurso del NAS
   - Rutas de logging       → ajustar al usuario local

4. Crear la tarea programada:
   C:\Sincronia\sincronia.exe --config C:\Sincronia\sincronia.toml --create-scheduled-task

5. Probar manualmente:
   C:\Sincronia\sincronia.exe --config C:\Sincronia\sincronia.toml

6. Cerrar sesión e iniciar sesión de nuevo → Sincronia arranca automáticamente
```

### Estructura de archivos en producción

```
C:\Sincronia\
├── sincronia.exe                    # Ejecutable
├── sincronia.toml                   # Configuración
└── (los logs se crean donde indica el TOML)

C:\Users\<usuario>\Documents\Sincronia\Logs\
├── sincronia.log                    # Log legible
├── sincronia-metrics.csv            # Métricas por archivo
└── sincronia-events.jsonl           # Eventos estructurados
```

---

## Arquitectura

### Diagrama de flujo del sistema

```
┌─────────────────────────────────────────────────────────────┐
│                    SINCRONIA - Flujo Principal               │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌──────────┐     ┌──────────────┐     ┌───────────────┐   │
│  │  main()  │────▶│ Orchestrator │────▶│  Worker Pool  │   │
│  │          │     │  (loop)      │     │  (N threads)  │   │
│  └──────────┘     └──────────────┘     └───────────────┘   │
│       │                  │                     │            │
│       ▼                  │                     │            │
│  ┌──────────┐           │                     │            │
│  │  Tray    │◀──────────┘                     │            │
│  │  (UI)    │  crossbeam channels             │            │
│  └──────────┘                                  │            │
│                                                ▼            │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Pipeline por archivo                     │   │
│  │                                                       │   │
│  │  Scan → Estabilidad → Conflictos → Copia → Hash      │   │
│  │                                     ▼        ▼        │   │
│  │                               Metadatos → Finalizar   │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Ciclo del orquestador

Cada iteración del loop principal sigue estos pasos:

1. **Validar NAS** — Verificar que `R:` apunta al UNC correcto
   - Si no está montada → intentar montar con Kerberos (sin credenciales)
   - Si Kerberos falla → mostrar diálogo nativo de credenciales de Windows
2. **Escanear** — Recorrer `source_directory_path` recursivamente, aplicando exclusiones
3. **Evaluar estabilidad** — Solo procesar archivos que no han cambiado en N segundos
4. **Planificar** — Crear `CopyJob` por cada archivo estable
5. **Ejecutar** — Enviar trabajos al pool de workers
6. **Registrar** — Métricas por archivo (.csv) y eventos (.jsonl)
7. **Esperar** — Dormir hasta el siguiente ciclo de escaneo

### Pipeline de copia (por archivo, en cada worker)

```
1. Verificar conflictos
   ├─ Archivo no existe en destino → copiar
   ├─ Existe con mismo hash → marcar como ya respaldado
   └─ Existe con hash diferente → crear copia versionada

2. Copiar a archivo temporal (.partial)
   - Buffer preasignado de 16 MiB por worker
   - FILE_FLAG_SEQUENTIAL_SCAN para optimizar I/O
   - FlushFileBuffers para garantizar escritura física

3. Verificar hash (BLAKE3)
   - Hash del origen vs hash del destino temporal
   - Si no coinciden → eliminar temporal, reintentar

4. Aplicar metadatos
   - Timestamps: creación, modificación, último acceso
   - Atributos: readonly, hidden, system, archive

5. Finalizar
   - Renombrar .partial → nombre final (atómico en NTFS)
   - Si modo "move" → eliminar origen

6. Reintentos
   - Hasta 3 intentos con delays de [2s, 5s, 15s]
   - Si todos fallan → marcar como SkippedAfterRetries
```

### Credenciales

Cuando el NAS necesita autenticación (Kerberos no disponible), Sincronia muestra el **diálogo nativo de credenciales de Windows** (`CredUIPromptForWindowsCredentialsW`). Es el mismo diálogo que aparece al mapear una unidad de red manualmente.

- Funciona sin consola (perfecto para tareas programadas)
- Las credenciales **nunca se almacenan en disco**
- La contraseña se sobrescribe en memoria al hacer `Drop`
- Si el usuario cancela → error transitorio, reintenta en el siguiente ciclo

### Bandeja del sistema (System Tray)

El icono cambia de color según el estado:

| Color | Significado |
|---|---|
| 🟢 Verde | NAS disponible, motor funcionando, inactivo esperando |
| 🟡 Amarillo | Iniciando, validando, esperando credenciales, pausado |
| 🔴 Rojo | Error transitorio/persistente, parando, parado |

Menú contextual (clic derecho):
- Estado actual (informativo)
- Montar NAS / Reintentar conexión
- Pausar / Reanudar / Parar ordenadamente
- Abrir carpeta de logs / configuración / métricas
- Salir

---

## Referencia de Módulos

| Módulo | Archivo | Función |
|---|---|---|
| **main** | `main.rs` | Punto de entrada. Parsea argumentos, carga config, lanza orquestador + tray |
| **config** | `config.rs` | Definición de todas las secciones del TOML con validación y defaults |
| **orchestrator** | `orchestrator.rs` | Máquina de estados principal: NAS → Scan → Estabilidad → Copia → Espera |
| **scanner** | `scanner.rs` | Recorrido recursivo del directorio origen con soporte de rutas largas (\\\\?\\) |
| **stability** | `stability.rs` | Detector: un archivo es estable si tamaño + mtime no cambian en N segundos |
| **planner** | `planner.rs` | Genera `CopyJob` con rutas origen/destino/temporal para cada archivo |
| **scheduler** | `scheduler.rs` | Pool de workers con buffers preasignados y canal crossbeam |
| **copy_engine** | `copy_engine.rs` | Copia por bloques con `FILE_FLAG_SEQUENTIAL_SCAN` y `FlushFileBuffers` |
| **verifier** | `verifier.rs` | Verificación post-copia: hash BLAKE3/SHA256 de origen vs destino |
| **hasher** | `hasher.rs` | Implementación de BLAKE3 y SHA-256 con buffer reutilizable |
| **metadata** | `metadata.rs` | Aplicar timestamps y atributos de archivo via API de Windows |
| **conflict** | `conflict.rs` | Resolución de conflictos: hash-compare, skip, versioned copy |
| **credentials** | `credentials.rs` | Diálogo nativo `CredUIPromptForWindowsCredentialsW` para credenciales NAS |
| **windows_nas** | `windows_nas.rs` | Montaje/desmontaje de unidad vía `WNetAddConnection2W` y fallback `net use` |
| **exclusions** | `exclusions.rs` | Filtros de exclusión por nombre de directorio y patrón glob de archivo |
| **logging** | `logging.rs` | Triple output: .log (humano), .csv (métricas), .jsonl (eventos) |
| **stats** | `stats.rs` | Agregador de métricas: contadores por ciclo, acumulados, velocidad media |
| **tray** | `tray.rs` | Icono en bandeja con menú contextual (tray-icon + winit) |
| **shutdown** | `shutdown.rs` | Señal de parada ordenada con `AtomicBool` + handler de Ctrl+C |
| **scheduled_task** | `scheduled_task.rs` | Creación de tarea programada vía script PowerShell |
| **errors** | `errors.rs` | Tipos de error (`SincroniaError`), estados globales, modos de ejecución |

---

## Solución de Problemas

### El diálogo de credenciales no aparece

- Verificar que `run_only_when_user_is_logged_on = true` en la sección `[scheduled_task]`
- La tarea debe ejecutarse en la sesión interactiva del usuario, no en segundo plano

### El NAS no se monta

- Verificar que la ruta UNC es accesible: `net view \\\\RAW-NAS\\Repositorio`
- Verificar DNS: `nslookup RAW-NAS`
- Si hay problemas con Kerberos, habilitar `allow_ip_fallback = true` y configurar `fallback_unc_path_by_ip`

### Los archivos no se copian

- Verificar `minimum_file_stable_seconds`: si es 60, el archivo debe estar quieto 60 segundos
- Verificar exclusiones: ¿el archivo coincide con algún patrón en `excluded_file_patterns`?
- Revisar logs en la ruta configurada en `human_log_file_path`

### Error "Se agotaron los intentos de credenciales"

- El usuario canceló el diálogo o introdujo credenciales incorrectas N veces
- Sincronia pasará a estado de error transitorio y reintentará en el siguiente ciclo
- Verificar que las credenciales son correctas: `net use R: \\\\RAW-NAS\\Repositorio /user:DOMINIO\usuario`

### La tarea programada no arranca

```powershell
# Verificar estado de la tarea
Get-ScheduledTask -TaskName "Sincronia NAS Backup" | Select-Object State, LastRunTime, LastTaskResult

# Ver el historial de ejecución
Get-ScheduledTaskInfo -TaskName "Sincronia NAS Backup"

# Ejecutar manualmente desde PowerShell
Start-ScheduledTask -TaskName "Sincronia NAS Backup"
```

### Ver logs en tiempo real

```powershell
Get-Content "C:\Users\thebe\Documents\Sincronia\Logs\sincronia.log" -Wait -Tail 50
```
