//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see [`__switch`]. Control flow around this function
//! might not be what you expect.


mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::config::{MAX_APP_NUM, MAX_SYSCALL_NUM};
use crate::loader::{get_num_app, init_app_cx};
use crate::sync::UPSafeCell;
use crate::syscall::TaskInfo;
use crate::timer::get_time_us;
use crate::syscall::{SYSCALL_WRITE, SYSCALL_EXIT,SYSCALL_YIELD, SYSCALL_GET_TIME, SYSCALL_TASK_INFO};

use lazy_static::*;
pub use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};


pub use context::TaskContext;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: [TaskControlBlock; MAX_APP_NUM],
    /// id of current `Running` task
    current_task: usize,
}


lazy_static! {
    /// a `TaskManager` instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
       // let empty_vec: [u32; MAX_SYSCALL_NUM] = [0; MAX_SYSCALL_NUM];
        let num_app = get_num_app();
       // println!("here");
        let mut tasks = [TaskControlBlock {
            task_cx: TaskContext::zero_init(),
            task_status: TaskStatus::UnInit,
            syscall_times: [0;5],

            start_time: 0,
        }; MAX_APP_NUM];
      //  println!("here2");

        for (i, t) in tasks.iter_mut().enumerate().take(num_app) {
            t.task_cx = TaskContext::goto_restore(init_app_cx(i));
            t.task_status = TaskStatus::Ready;
        }

       let ret = TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks: tasks,
                    current_task: 0,
                })
            },
        };
       // println!("ready");
        ret
    };
   

}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch3, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
      //  println!("run first task");
        let mut inner = self.inner.exclusive_access();
        let task0 = &mut inner.tasks[0];
        task0.task_status = TaskStatus::Running;
        task0.start_time = get_time_us()/1000;
        let next_task_cx_ptr = &task0.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut TaskContext, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Ready;
      //  inner.tasks[current].time += get_time_us()/1000 - inner.tasks[current].pre_start_time;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Exited;
       // inner.tasks[current].time += get_time_us()/1000 - inner.tasks[current].pre_start_time;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            if inner.tasks[next].start_time == 0 { inner.tasks[next].start_time = get_time_us()/1000;}
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }

    // LAB1: Try to implement your function to update or get task info!

    fn update_syscall_num(&self, syscall_id: usize){
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].syscall_times[map_syscall_to_small_range(syscall_id)] += 1;
        drop(inner);
    }

}

fn map_syscall_to_small_range(syscall_id: usize) ->usize{
    match syscall_id{
        SYSCALL_WRITE => 0,
        SYSCALL_YIELD => 1,
        SYSCALL_EXIT => 2,
        SYSCALL_GET_TIME => 3,
        SYSCALL_TASK_INFO => 4,
        _ => todo!(),
    }
}

fn map_small_range_to_syscall(id: usize) -> usize{
    match id{
        0 => SYSCALL_WRITE,
        1 => SYSCALL_YIELD,
        2 => SYSCALL_EXIT,
        3 => SYSCALL_GET_TIME,
        4 => SYSCALL_TASK_INFO,
        _ => todo!(),
    }
}

/// Run the first task in task list.
pub fn run_first_task() {
  //  println!("ready to run first");
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

// LAB1: Public functions implemented here provide interfaces.
// You may use TASK_MANAGER member functions to handle requests.

pub fn update_current_syscall_times(syscall_id: usize){
    TASK_MANAGER.update_syscall_num(syscall_id);
}

pub fn get_current_task_info(ti: &mut TaskInfo) -> isize {
    let inner = TASK_MANAGER.inner.exclusive_access();
    let current = inner.current_task;
    let cur_task = inner.tasks[current];
    ti.status = cur_task.task_status;
    ti.syscall_times = [0; MAX_SYSCALL_NUM];
    for i in 0..5{
        ti.syscall_times[map_small_range_to_syscall(i)]= cur_task.syscall_times[i];
    }
   // ti.syscall_times = cur_task.syscall_times;
    ti.time = get_time_us()/1000 - cur_task.start_time;
    0
}