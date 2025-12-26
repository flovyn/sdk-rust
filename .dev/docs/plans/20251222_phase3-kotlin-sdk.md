# Phase 3: Kotlin SDK - Implementation Plan

## Overview

This plan implements Phase 3 from the [Core SDK in Rust for Multiple Languages](../design/core-sdk-in-rust-for-multiple-language.md) design document.

**Goal**: Create a new `sdk-kotlin` repository that uses the Rust FFI core (`flovyn-ffi`) while providing an idiomatic Kotlin API inspired by the [existing Kotlin SDK](file:///Users/manhha/Developer/manhha/leanapp/flovyn/sdk/kotlin).

**Constraint**: The API should feel native to Kotlin developers - no annotations required, just class inheritance and property overrides.

## Prerequisites

- Phase 2 complete (ffi crate with uniffi bindings)
- Familiarity with the existing Kotlin SDK API design
- Kotlin 1.9+, JDK 17+

## API Design Principles (from existing SDK)

The existing Kotlin SDK has an elegant, annotation-free design:

1. **Class inheritance** - Extend `WorkflowDefinition<I, O>` or `TaskDefinition<I, O>`
2. **Property overrides** - Configure via `override val kind`, `version`, etc.
3. **Suspend functions** - Kotlin coroutines for async operations
4. **Builder pattern** - `FlovynClient.builder()...build()`
5. **Reified generics** - Type-safe scheduling with `inline fun <reified T>`
6. **No annotations** - Jackson serialization works with plain data classes

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      User Application                            │
│                                                                  │
│  class OrderWorkflow : WorkflowDefinition<OrderInput, OrderOutput>() {
│      override val kind = "order-workflow"                        │
│      override suspend fun execute(ctx: WorkflowContext, input: OrderInput) = ...
│  }                                                               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      sdk-kotlin                                  │
│                                                                  │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐       │
│  │ FlovynClient  │  │ Workflow      │  │ Task          │       │
│  │ Builder       │  │ Definition    │  │ Definition    │       │
│  └───────────────┘  └───────────────┘  └───────────────┘       │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐       │
│  │ Workflow      │  │ Task          │  │ Registry      │       │
│  │ Context       │  │ Context       │  │               │       │
│  └───────────────┘  └───────────────┘  └───────────────┘       │
│                              │                                   │
│                    Activation Protocol                          │
│                              │                                   │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │              CoreBridge (wraps FFI)                       │  │
│  │  - Converts Kotlin types ↔ FFI activation types           │  │
│  │  - Manages tokio runtime on Rust side                     │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ JNI / uniffi
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      flovyn-ffi (Rust)                           │
│                                                                  │
│  CoreWorker, CoreClient, Activation types                       │
└─────────────────────────────────────────────────────────────────┘
```

## Repository Structure

```
sdk-kotlin/                           # New Git repository
├── .github/
│   └── workflows/
│       ├── ci.yml                    # PR checks, build, test
│       └── release.yml               # Publish to Maven Central
│
├── core/                             # Core module (wraps FFI)
│   ├── build.gradle.kts
│   └── src/main/kotlin/io/flovyn/core/
│       ├── CoreBridge.kt             # FFI bridge wrapper
│       ├── CoreWorker.kt             # Wraps FFI CoreWorker
│       ├── Activation.kt             # Kotlin activation types
│       └── Runtime.kt                # Runtime initialization
│
├── sdk-jackson/                      # SDK with Jackson serialization
│   ├── build.gradle.kts
│   └── src/
│       ├── main/kotlin/io/flovyn/sdk/
│       │   ├── workflow/
│       │   │   ├── WorkflowDefinition.kt
│       │   │   ├── WorkflowContext.kt
│       │   │   ├── WorkflowContextImpl.kt
│       │   │   ├── Deferred.kt
│       │   │   └── Command.kt
│       │   ├── task/
│       │   │   ├── TaskDefinition.kt
│       │   │   ├── TaskContext.kt
│       │   │   ├── TaskContextImpl.kt
│       │   │   └── RetryConfig.kt
│       │   ├── client/
│       │   │   ├── FlovynClient.kt
│       │   │   ├── FlovynClientBuilder.kt
│       │   │   └── WorkflowHook.kt
│       │   ├── worker/
│       │   │   ├── WorkflowWorker.kt
│       │   │   ├── TaskWorker.kt
│       │   │   ├── WorkflowRegistry.kt
│       │   │   └── TaskRegistry.kt
│       │   └── serialization/
│       │       └── JacksonStrategy.kt
│       └── test/kotlin/              # Unit tests
│           └── io/flovyn/sdk/
│
├── native/                           # Native library loader
│   ├── build.gradle.kts
│   └── src/main/resources/
│       └── natives/
│           ├── linux-x86_64/         # libflovyn_ffi.so
│           ├── macos-x86_64/         # libflovyn_ffi.dylib
│           └── macos-aarch64/        # libflovyn_ffi.dylib
│
├── examples/                         # Example applications
│   └── src/main/kotlin/
│       ├── HelloWorld.kt
│       └── OrderProcessing.kt
│
├── build.gradle.kts                  # Root build file
├── settings.gradle.kts
├── gradle.properties
└── README.md
```

## TODO List

### Step 1: Create Repository and Build Setup

- [x] Create `sdk-kotlin` repository
- [x] Create root `build.gradle.kts` with Kotlin configuration
- [x] Create `settings.gradle.kts` with module structure
- [x] Add Kotlin, coroutines, Jackson dependencies
- [x] Verify: `./gradlew build` passes (empty project)

```kotlin
// build.gradle.kts
plugins {
    kotlin("jvm") version "1.9.22"
}

allprojects {
    repositories {
        mavenCentral()
    }
}

subprojects {
    apply(plugin = "kotlin")

    dependencies {
        implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0")
        testImplementation(kotlin("test"))
    }
}
```

### Step 2: Create Native Module with FFI Bindings

- [x] Create `native/` module
- [x] Add uniffi-generated Kotlin bindings from Phase 2
- [x] Create native library loader for platform-specific libraries
- [x] Add pre-built native libraries (.dylib for macOS, .so for Linux)
- [x] Verify: Native library loads successfully

```kotlin
// native/src/main/kotlin/io/flovyn/native/NativeLoader.kt
object NativeLoader {
    init {
        val os = System.getProperty("os.name").lowercase()
        val arch = System.getProperty("os.arch")
        val libName = when {
            os.contains("mac") -> "libflovyn_ffi.dylib"
            os.contains("linux") -> "libflovyn_ffi.so"
            os.contains("windows") -> "flovyn_ffi.dll"
            else -> error("Unsupported OS: $os")
        }
        // Load from resources or system path
        System.loadLibrary("flovyn_ffi")
    }
}
```

### Step 3: Create Core Module - Bridge Layer

- [x] Create `core/` module
- [x] Create `CoreBridge.kt` that wraps uniffi-generated classes
- [x] Create Kotlin-native activation types (mirror FFI types)
- [x] Implement conversion between Kotlin and FFI types
- [x] Verify: Core module compiles

```kotlin
// core/src/main/kotlin/io/flovyn/core/CoreBridge.kt
class CoreBridge private constructor(
    private val ffiWorker: uniffi.flovyn_ffi.CoreWorker
) {
    suspend fun pollWorkflowActivation(): WorkflowActivation {
        val ffiActivation = ffiWorker.pollWorkflowActivation()
        return ffiActivation.toKotlin()
    }

    suspend fun completeWorkflowActivation(completion: WorkflowActivationCompletion) {
        ffiWorker.completeWorkflowActivation(completion.toFfi())
    }

    suspend fun pollTaskActivation(): TaskActivation {
        val ffiActivation = ffiWorker.pollTaskActivation()
        return ffiActivation.toKotlin()
    }

    suspend fun completeTask(completion: TaskCompletion) {
        ffiWorker.completeTask(completion.toFfi())
    }

    fun shutdown() {
        ffiWorker.initiateShutdown()
    }

    companion object {
        suspend fun create(config: WorkerConfig): CoreBridge {
            val ffiWorker = uniffi.flovyn_ffi.CoreWorker.new(config.toFfi())
            return CoreBridge(ffiWorker)
        }
    }
}
```

### Step 4: Create WorkflowDefinition Base Class

- [x] Create `sdk-jackson/` module
- [x] Create `WorkflowDefinition.kt` abstract class
- [x] Define `kind`, `name`, `version`, `description` properties
- [x] Define `execute(ctx, input)` abstract suspend function
- [x] Create `DynamicWorkflowDefinition` typealias
- [x] Verify: Can create workflow subclass

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/workflow/WorkflowDefinition.kt
abstract class WorkflowDefinition<INPUT, OUTPUT> {
    abstract val kind: String

    open val name: String get() = kind
    open val version: SemanticVersion get() = SemanticVersion(1, 0, 0)
    open val description: String? get() = null
    open val timeoutSeconds: Int? get() = null
    open val cancellable: Boolean get() = true
    open val tags: List<String> get() = emptyList()

    abstract suspend fun execute(ctx: WorkflowContext, input: INPUT): OUTPUT
}

// Convenience alias for dynamic workflows
typealias DynamicWorkflowDefinition = WorkflowDefinition<Map<String, Any?>, Map<String, Any?>>

data class SemanticVersion(
    val major: Int,
    val minor: Int,
    val patch: Int
) {
    override fun toString() = "$major.$minor.$patch"
}
```

### Step 5: Create WorkflowContext Interface

- [x] Create `WorkflowContext.kt` interface
- [x] Add deterministic time/random methods
- [x] Add `schedule<T>()` with reified generics
- [x] Add `scheduleAsync<T>()` returning `Deferred<T>`
- [x] Add state management methods (`get`, `set`, `clear`)
- [x] Add `promise()` for external signals
- [x] Add `sleep()` for durable timers
- [x] Verify: Interface compiles

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/workflow/WorkflowContext.kt
interface WorkflowContext {
    val workflowExecutionId: UUID
    val tenantId: UUID
    val input: Map<String, Any?>

    // Deterministic operations
    fun currentTimeMillis(): Long
    fun randomUUID(): UUID
    fun random(): Random

    // Task scheduling (typed)
    suspend inline fun <reified T : Any> schedule(
        taskType: String,
        input: Any?,
        options: ScheduleTaskOptions = ScheduleTaskOptions.DEFAULT
    ): T

    // Task scheduling (async)
    suspend inline fun <reified T : Any> scheduleAsync(
        taskType: String,
        input: Any?,
        options: ScheduleTaskOptions = ScheduleTaskOptions.DEFAULT
    ): Deferred<T>

    // Operations (recorded for replay)
    suspend fun <T> run(name: String, block: suspend () -> T): T
    suspend fun <T> runAsync(name: String, block: suspend () -> T): Deferred<T>

    // Child workflows
    suspend fun <T> scheduleWorkflow(
        workflowKind: String,
        childExecutionName: String,
        input: Any?,
        options: ScheduleWorkflowOptions = ScheduleWorkflowOptions.DEFAULT
    ): T

    // State management
    suspend fun <T> get(key: String): T?
    suspend fun <T> set(key: String, value: T)
    suspend fun clear(key: String)
    suspend fun clearAll()
    suspend fun stateKeys(): List<String>

    // Durable promises
    suspend fun <T> promise(name: String, timeout: Duration? = null): DurablePromise<T>

    // Timers
    suspend fun sleep(duration: Duration)

    // Cancellation
    fun isCancellationRequested(): Boolean
    suspend fun checkCancellation()
}

data class ScheduleTaskOptions(
    val timeoutSeconds: Int? = null,
    val retryConfig: RetryConfig? = null
) {
    companion object {
        val DEFAULT = ScheduleTaskOptions()
    }
}
```

### Step 6: Create WorkflowContextImpl

- [x] Create `WorkflowContextImpl.kt` implementing `WorkflowContext`
- [x] Track commands generated during execution
- [x] Implement replay logic using existing events
- [x] Implement deterministic random using seeded RNG
- [x] Implement `Deferred<T>` for async operations
- [x] Verify: Context correctly generates commands

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/workflow/WorkflowContextImpl.kt
internal class WorkflowContextImpl(
    override val workflowExecutionId: UUID,
    override val tenantId: UUID,
    override val input: Map<String, Any?>,
    private val timestampMs: Long,
    private val randomSeed: ByteArray,
    private val existingEvents: List<ReplayEvent>,
    private val serializer: JsonSerializer
) : WorkflowContext {

    private val commands = mutableListOf<WorkflowCommand>()
    private var sequenceNumber = 0
    private val random = SeededRandom(randomSeed)

    // Event caches for O(1) lookup during replay
    private val taskEvents: Map<String, ReplayEvent>
    private val timerEvents: Map<String, ReplayEvent>
    private val operationEvents: Map<String, ReplayEvent>
    // ... etc

    override fun currentTimeMillis(): Long = timestampMs

    override fun randomUUID(): UUID = random.nextUUID()

    override suspend inline fun <reified T : Any> schedule(
        taskType: String,
        input: Any?,
        options: ScheduleTaskOptions
    ): T {
        val taskId = "task-${++sequenceNumber}"

        // Check for existing result during replay
        taskEvents[taskId]?.let { event ->
            return serializer.deserialize(event.result, T::class.java)
        }

        // Record command for new execution
        commands.add(WorkflowCommand.ScheduleTask(
            sequenceNumber = sequenceNumber,
            taskType = taskType,
            taskExecutionId = taskId,
            input = serializer.serialize(input)
        ))

        // Suspend until task completes (handled by executor)
        return suspendCancellableCoroutine { cont ->
            pendingTasks[taskId] = cont
        }
    }

    fun getCommands(): List<WorkflowCommand> = commands.toList()
}
```

### Step 7: Create TaskDefinition Base Class

- [x] Create `TaskDefinition.kt` abstract class
- [x] Define properties: `kind`, `name`, `retryConfig`, `timeoutSeconds`
- [x] Define `execute(input, ctx)` abstract suspend function
- [x] Create `DynamicTaskDefinition` typealias
- [x] Verify: Can create task subclass

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/task/TaskDefinition.kt
abstract class TaskDefinition<INPUT, OUTPUT> {
    abstract val kind: String

    open val name: String get() = kind
    open val version: SemanticVersion get() = SemanticVersion(1, 0, 0)
    open val description: String? get() = null
    open val timeoutSeconds: Int? get() = null
    open val cancellable: Boolean get() = true
    open val tags: List<String> get() = emptyList()
    open val heartbeatTimeoutSeconds: Int? get() = null
    open val retryConfig: RetryConfig get() = RetryConfig.DEFAULT

    abstract suspend fun execute(input: INPUT, context: TaskContext): OUTPUT
}

typealias DynamicTaskDefinition = TaskDefinition<Map<String, Any?>, Map<String, Any?>>

data class RetryConfig(
    val maxAttempts: Int = 3,
    val initialDelayMs: Long = 1000,
    val maxDelayMs: Long = 60000,
    val backoffMultiplier: Double = 2.0
) {
    companion object {
        val DEFAULT = RetryConfig()
    }
}
```

### Step 8: Create TaskContext Interface and Implementation

- [x] Create `TaskContext.kt` interface
- [x] Add `reportProgress()`, `heartbeat()`, `log()`
- [x] Add cancellation methods
- [x] Add streaming support (`stream()`)
- [x] Add state management for idempotency
- [x] Create `TaskContextImpl.kt`
- [x] Verify: Task context works

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/task/TaskContext.kt
interface TaskContext {
    val taskExecutionId: String
    val input: Any?
    val attempt: Int

    // Progress and monitoring
    suspend fun reportProgress(progress: Double, details: String? = null)
    suspend fun heartbeat()
    suspend fun log(level: String, message: String)

    // Cancellation
    fun isCancelled(): Boolean
    fun checkCancellation()

    // Streaming (ephemeral)
    suspend fun stream(event: StreamEvent)

    // Persistent state
    suspend fun <T> get(key: String): T?
    suspend fun <T> set(key: String, value: T)
    suspend fun clear(key: String)
    suspend fun stateKeys(): Set<String>
}

sealed class StreamEvent {
    data class Token(val text: String) : StreamEvent()
    data class Progress(val progress: Double, val details: String? = null) : StreamEvent()
    data class Data(val data: String) : StreamEvent()
    data class Error(val message: String, val code: String? = null) : StreamEvent()
}
```

### Step 9: Create Registry Classes

- [x] Create `WorkflowRegistry.kt` with reified registration
- [x] Create `TaskRegistry.kt` with reified registration
- [x] Support both typed and dynamic registrations
- [x] Extract metadata for server registration
- [x] Verify: Registries work correctly

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/worker/WorkflowRegistry.kt
class WorkflowRegistry {
    private val workflows = mutableMapOf<String, RegisteredWorkflow<*, *>>()

    inline fun <reified INPUT, reified OUTPUT> register(
        workflow: WorkflowDefinition<INPUT, OUTPUT>
    ) {
        workflows[workflow.kind] = RegisteredWorkflow(
            definition = workflow,
            inputType = INPUT::class.java,
            outputType = OUTPUT::class.java
        )
    }

    fun registerDynamic(workflow: DynamicWorkflowDefinition) {
        workflows[workflow.kind] = RegisteredWorkflow(
            definition = workflow,
            inputType = Map::class.java,
            outputType = Map::class.java
        )
    }

    fun get(kind: String): RegisteredWorkflow<*, *>? = workflows[kind]
    fun has(kind: String): Boolean = kind in workflows
    fun getAllMetadata(): List<WorkflowMetadata> = workflows.values.map { it.toMetadata() }
}

data class RegisteredWorkflow<INPUT, OUTPUT>(
    val definition: WorkflowDefinition<INPUT, OUTPUT>,
    val inputType: Class<*>,
    val outputType: Class<*>
)
```

### Step 10: Create FlovynClientBuilder

- [x] Create `FlovynClientBuilder.kt` with fluent API
- [x] Add server connection configuration
- [x] Add workflow/task registration methods
- [x] Add hook registration
- [x] Add concurrency configuration
- [x] Verify: Builder creates valid client

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/client/FlovynClientBuilder.kt
class FlovynClientBuilder {
    private var serverAddress: String = "localhost"
    private var serverPort: Int = 9090
    private var tenantId: UUID? = null
    private var workerId: String? = null
    private var taskQueue: String = "default"
    private var maxConcurrentWorkflows: Int = 10
    private var maxConcurrentTasks: Int = 20

    private val workflowRegistry = WorkflowRegistry()
    private val taskRegistry = TaskRegistry()
    private val hooks = mutableListOf<WorkflowHook>()

    fun serverAddress(host: String, port: Int) = apply {
        this.serverAddress = host
        this.serverPort = port
    }

    fun tenantId(tenantId: UUID) = apply { this.tenantId = tenantId }
    fun workerId(workerId: String) = apply { this.workerId = workerId }
    fun taskQueue(taskQueue: String) = apply { this.taskQueue = taskQueue }

    fun maxConcurrentWorkflows(max: Int) = apply { this.maxConcurrentWorkflows = max }
    fun maxConcurrentTasks(max: Int) = apply { this.maxConcurrentTasks = max }

    inline fun <reified INPUT, reified OUTPUT> registerWorkflow(
        workflow: WorkflowDefinition<INPUT, OUTPUT>
    ) = apply {
        workflowRegistry.register(workflow)
    }

    fun registerWorkflow(workflow: DynamicWorkflowDefinition) = apply {
        workflowRegistry.registerDynamic(workflow)
    }

    inline fun <reified INPUT, reified OUTPUT> registerTask(
        task: TaskDefinition<INPUT, OUTPUT>
    ) = apply {
        taskRegistry.register(task)
    }

    fun registerTask(task: DynamicTaskDefinition) = apply {
        taskRegistry.registerDynamic(task)
    }

    fun registerHook(hook: WorkflowHook) = apply { hooks.add(hook) }

    suspend fun build(): FlovynClient {
        requireNotNull(tenantId) { "tenantId is required" }

        val config = WorkerConfig(
            serverUrl = "$serverAddress:$serverPort",
            tenantId = tenantId.toString(),
            taskQueue = taskQueue,
            workerIdentity = workerId,
            maxConcurrentWorkflowTasks = maxConcurrentWorkflows,
            maxConcurrentTasks = maxConcurrentTasks
        )

        val coreBridge = CoreBridge.create(config)

        return FlovynClient(
            coreBridge = coreBridge,
            workflowRegistry = workflowRegistry,
            taskRegistry = taskRegistry,
            hooks = CompositeWorkflowHook(hooks),
            config = config
        )
    }
}
```

### Step 11: Create FlovynClient

- [x] Create `FlovynClient.kt` main class
- [x] Implement `start()`, `stop()`, `awaitReady()`
- [x] Implement `startWorkflow()` methods
- [x] Implement `query()`, `resolvePromise()`, `rejectPromise()`
- [x] Coordinate workflow and task workers
- [x] Verify: Client can start and stop

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/client/FlovynClient.kt
class FlovynClient internal constructor(
    private val coreBridge: CoreBridge,
    private val workflowRegistry: WorkflowRegistry,
    private val taskRegistry: TaskRegistry,
    private val hooks: CompositeWorkflowHook,
    private val config: WorkerConfig
) {
    private val workflowWorker = WorkflowWorker(coreBridge, workflowRegistry, hooks)
    private val taskWorker = TaskWorker(coreBridge, taskRegistry)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    fun hasWorkflow(kind: String): Boolean = workflowRegistry.has(kind)
    fun hasTask(type: String): Boolean = taskRegistry.has(type)

    fun start() {
        scope.launch { workflowWorker.run() }
        scope.launch { taskWorker.run() }
    }

    suspend fun awaitReady() {
        // Wait for workers to start polling
    }

    fun stop() {
        coreBridge.shutdown()
        scope.cancel()
    }

    suspend fun startWorkflow(
        workflowKind: String,
        input: Any? = null,
        options: StartWorkflowOptions = StartWorkflowOptions()
    ): UUID {
        // Use CoreClient to start workflow
    }

    suspend fun <T> query(
        workflowExecutionId: UUID,
        queryName: String,
        params: Map<String, Any?> = emptyMap()
    ): T {
        // Query workflow state
    }

    suspend fun resolvePromise(
        workflowExecutionId: UUID,
        promiseName: String,
        value: Any?
    ) {
        // Resolve durable promise
    }

    companion object {
        fun builder(): FlovynClientBuilder = FlovynClientBuilder()
    }
}
```

### Step 12: Create WorkflowWorker

- [x] Create `WorkflowWorker.kt` polling loop
- [x] Poll activations from CoreBridge
- [x] Execute workflow with WorkflowContextImpl
- [x] Submit completion back to CoreBridge
- [x] Handle errors and retries
- [x] Verify: Workflow execution works

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/worker/WorkflowWorker.kt
internal class WorkflowWorker(
    private val coreBridge: CoreBridge,
    private val registry: WorkflowRegistry,
    private val hooks: CompositeWorkflowHook
) {
    suspend fun run() {
        while (true) {
            try {
                val activation = coreBridge.pollWorkflowActivation()
                processActivation(activation)
            } catch (e: ShutdownException) {
                break
            } catch (e: Exception) {
                // Log and continue
            }
        }
    }

    private suspend fun processActivation(activation: WorkflowActivation) {
        val workflow = registry.get(activation.workflowKind)
            ?: throw UnknownWorkflowException(activation.workflowKind)

        val context = WorkflowContextImpl(
            workflowExecutionId = activation.workflowExecutionId,
            tenantId = activation.tenantId,
            input = activation.input,
            timestampMs = activation.timestampMs,
            randomSeed = activation.randomSeed,
            existingEvents = activation.existingEvents
        )

        try {
            hooks.onWorkflowStarted(activation.workflowExecutionId, activation.workflowKind, activation.input)

            val result = executeWorkflow(workflow, context, activation.input)

            val completion = WorkflowActivationCompletion(
                runId = activation.runId,
                commands = context.getCommands() + WorkflowCommand.CompleteWorkflow(result)
            )

            coreBridge.completeWorkflowActivation(completion)
            hooks.onWorkflowCompleted(activation.workflowExecutionId, activation.workflowKind, result)

        } catch (e: Exception) {
            val completion = WorkflowActivationCompletion(
                runId = activation.runId,
                commands = listOf(WorkflowCommand.FailWorkflow(e.message ?: "Unknown error"))
            )
            coreBridge.completeWorkflowActivation(completion)
            hooks.onWorkflowFailed(activation.workflowExecutionId, activation.workflowKind, e)
        }
    }
}
```

### Step 13: Create TaskWorker

- [x] Create `TaskWorker.kt` polling loop
- [x] Poll task activations from CoreBridge
- [x] Execute task with TaskContextImpl
- [x] Submit completion back to CoreBridge
- [ ] Handle streaming events
- [x] Verify: Task execution works

### Step 14: Create WorkflowHook Interface

- [x] Create `WorkflowHook.kt` interface
- [x] Add lifecycle callbacks
- [x] Create `CompositeWorkflowHook` for multiple hooks
- [x] Verify: Hooks are called correctly

```kotlin
// sdk-jackson/src/main/kotlin/io/flovyn/sdk/client/WorkflowHook.kt
interface WorkflowHook {
    suspend fun onWorkflowStarted(
        workflowExecutionId: UUID,
        workflowKind: String,
        input: Map<String, Any?>
    ) {}

    suspend fun onWorkflowCompleted(
        workflowExecutionId: UUID,
        workflowKind: String,
        result: Any?
    ) {}

    suspend fun onWorkflowFailed(
        workflowExecutionId: UUID,
        workflowKind: String,
        error: Throwable
    ) {}

    suspend fun onTaskScheduled(
        workflowExecutionId: UUID,
        taskId: String,
        taskType: String,
        input: Any?
    ) {}
}
```

### Step 15: Create Examples

- [ ] Create `examples/` module
- [ ] Create HelloWorld example
- [ ] Create OrderProcessing example with tasks
- [ ] Add README with usage instructions
- [ ] Verify: Examples compile and run

**Status**: Deferred - SDK is functional, examples can be added later.

```kotlin
// examples/src/main/kotlin/HelloWorld.kt
class GreetingWorkflow : WorkflowDefinition<GreetingInput, GreetingOutput>() {
    override val kind = "greeting-workflow"
    override val version = SemanticVersion(1, 0, 0)

    override suspend fun execute(ctx: WorkflowContext, input: GreetingInput): GreetingOutput {
        val message = "Hello, ${input.name}!"
        ctx.set("greeting", message)
        return GreetingOutput(message = message, timestamp = ctx.currentTimeMillis())
    }
}

data class GreetingInput(val name: String)
data class GreetingOutput(val message: String, val timestamp: Long)

suspend fun main() {
    val client = FlovynClient.builder()
        .serverAddress("localhost", 9090)
        .tenantId(UUID.randomUUID())
        .registerWorkflow(GreetingWorkflow())
        .build()

    client.start()
    client.awaitReady()

    val executionId = client.startWorkflow("greeting-workflow", GreetingInput("World"))
    println("Started workflow: $executionId")

    // Wait and stop
    delay(5000)
    client.stop()
}
```

### Step 16: Create E2E Tests

- [x] Create test module with E2E tests
- [x] Test basic workflow execution
- [x] Test task scheduling
- [x] Test state management
- [x] Test durable promises
- [x] Test deterministic random (ctx.random(), ctx.randomUUID())
- [x] Test durable timers (ctx.sleep())
- [x] Use different task queues per test
- [x] Verify: All E2E tests pass

### Step 17: GitHub Actions CI/CD

- [ ] Create `.github/workflows/ci.yml` for PR checks
- [ ] Create `.github/workflows/release.yml` for publishing
- [ ] Set up Gradle caching
- [ ] Build native libraries for multiple platforms (macOS, Linux)
- [ ] Run tests against Flovyn server (using docker-compose or dev infra)
- [ ] Verify: CI passes on push/PR

**Status**: Deferred - CI/CD can be configured once SDK stabilizes.

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Set up JDK 17
        uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'

      - name: Setup Gradle
        uses: gradle/actions/setup-gradle@v3
        with:
          cache-read-only: ${{ github.ref != 'refs/heads/main' }}

      - name: Build
        run: ./gradlew build

      - name: Run unit tests
        run: ./gradlew test

  e2e-test:
    runs-on: ubuntu-latest
    needs: build

    services:
      flovyn-server:
        image: flovyn/server:latest
        ports:
          - 9090:9090
        options: >-
          --health-cmd "grpc_health_probe -addr=:9090"
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5

    steps:
      - uses: actions/checkout@v4

      - name: Set up JDK 17
        uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'

      - name: Setup Gradle
        uses: gradle/actions/setup-gradle@v3

      - name: Run E2E tests
        run: ./gradlew :sdk-jackson:e2eTest
        env:
          FLOVYN_SERVER_ADDRESS: localhost:9090

  native-build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            lib: libflovyn_ffi.so
          - os: macos-latest
            target: x86_64-apple-darwin
            lib: libflovyn_ffi.dylib
          - os: macos-latest
            target: aarch64-apple-darwin
            lib: libflovyn_ffi.dylib

    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-action@stable
        with:
          targets: ${{ matrix.target }}

      - name: Checkout sdk-rust (for ffi crate)
        uses: actions/checkout@v4
        with:
          repository: flovyn/sdk-rust
          path: sdk-rust

      - name: Build native library
        run: |
          cd sdk-rust
          cargo build -p flovyn-ffi --release --target ${{ matrix.target }}

      - name: Upload native library
        uses: actions/upload-artifact@v4
        with:
          name: native-${{ matrix.target }}
          path: sdk-rust/target/${{ matrix.target }}/release/${{ matrix.lib }}
```

```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build-natives:
    # Same as native-build job above
    ...

  publish:
    needs: build-natives
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Download all native libraries
        uses: actions/download-artifact@v4
        with:
          path: native/src/main/resources/natives/

      - name: Set up JDK 17
        uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'

      - name: Publish to Maven Central
        run: ./gradlew publish
        env:
          MAVEN_USERNAME: ${{ secrets.MAVEN_USERNAME }}
          MAVEN_PASSWORD: ${{ secrets.MAVEN_PASSWORD }}
          SIGNING_KEY: ${{ secrets.SIGNING_KEY }}
          SIGNING_PASSWORD: ${{ secrets.SIGNING_PASSWORD }}
```

### Step 18: Documentation and Cleanup

- [ ] Add KDoc to public APIs
- [ ] Create README.md with quick start guide
- [ ] Add API usage examples
- [ ] Document serialization requirements
- [ ] Document native library requirements
- [ ] Verify: Documentation is complete

**Status**: Deferred - Documentation can be added once API stabilizes.

### Step 19: Final Verification

- [x] `./gradlew build` passes
- [x] `./gradlew test` passes
- [ ] Examples run successfully against server
- [x] E2E tests pass
- [ ] GitHub Actions CI passes
- [x] API matches existing Kotlin SDK style

## API Comparison

| Feature | Existing SDK | New SDK (with FFI) |
|---------|--------------|-------------------|
| Workflow definition | `class X : WorkflowDefinition<I, O>` | Same |
| Task definition | `class X : TaskDefinition<I, O>` | Same |
| Client builder | `FlovynClient.builder()...build()` | Same |
| Workflow registration | `.registerWorkflow(X())` | Same |
| Context scheduling | `ctx.schedule<T>("task", input)` | Same |
| Async scheduling | `ctx.scheduleAsync<T>(...)` | Same |
| State management | `ctx.get/set/clear` | Same |
| Promises | `ctx.promise("name")` | Same |
| Hooks | `WorkflowHook` interface | Same |
| Serialization | Jackson (no annotations) | Same |

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| JNI complexity | uniffi handles JNI generation |
| Native library distribution | Bundle in JAR resources; platform-specific classifiers |
| Coroutine bridging | uniffi async maps to suspend functions |
| Serialization mismatch | Use JSON (Vec<u8>) at FFI boundary; serialize in Kotlin |
| Type erasure | Use reified generics in inline functions |
| Replay determinism | Core handles replay; Kotlin only generates commands |

## Success Criteria

1. API is annotation-free (class inheritance + property overrides)
2. Workflows can be defined as `WorkflowDefinition<I, O>` subclasses
3. Tasks can be defined as `TaskDefinition<I, O>` subclasses
4. `FlovynClient.builder()` provides fluent configuration
5. `ctx.schedule<T>()` works with reified generics
6. E2E tests pass against real server
7. No Jackson annotations required on data classes
8. GitHub Actions CI/CD pipeline works (build, test, native library builds)
9. Native libraries built for Linux (x86_64) and macOS (x86_64, aarch64)
