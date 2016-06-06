pub use super::super::compiler::*;

use super::postprocess;
use super::super::utils::filter;
use super::super::io::memstream::MemStream;
use super::super::io::tempfile::TempFile;

use std::fs::File;
use std::io::{Error, Cursor, Write};
use std::path::{Path, PathBuf};

pub struct VsCompiler {
	temp_dir: PathBuf
}

impl VsCompiler {
	pub fn new(temp_dir: &Path) -> Self {
		VsCompiler {
			temp_dir: temp_dir.to_path_buf()
		}
	}
}

impl Compiler for VsCompiler {
	fn create_task(&self, command: CommandInfo, args: &[String]) -> Result<Option<CompilationTask>, String> {
		super::prepare::create_task(command, args)
	}

	fn preprocess_step(&self, task: &CompilationTask) -> Result<PreprocessResult, Error> {
		// Make parameters list for preprocessing.
		let mut args = filter(&task.args, |arg:&Arg|->Option<String> {
			match arg {
				&Arg::Flag{ref scope, ref flag} => {
					match scope {
						&Scope::Preprocessor | &Scope::Shared => Some("/".to_string() + &flag),
						&Scope::Ignore | &Scope::Compiler => None
					}
				}
				&Arg::Param{ref scope, ref  flag, ref value} => {
					match scope {
						&Scope::Preprocessor | &Scope::Shared => Some("/".to_string() + &flag + &value),
						&Scope::Ignore | &Scope::Compiler => None
					}
				}
				&Arg::Input{..} => None,
				&Arg::Output{..} => None,
			}
		});
	
		// Add preprocessor paramters.
		args.push("/nologo".to_string());
		args.push("/T".to_string() + &task.language);
		args.push("/E".to_string());
		args.push("/we4002".to_string()); // C4002: too many actual parameters for macro 'identifier'
		args.push(task.input_source.display().to_string());

		let mut command = task.command.to_command();
		command
			.args(&args)
			.arg(&join_flag("/Fo", &task.output_object)); // /Fo option also set output path for #import directive
		let output = try! (command.output());
		if output.status.success() {
			let mut content = MemStream::new();
			if task.input_precompiled.is_some() || task.output_precompiled.is_some() {
				try! (postprocess::filter_preprocessed(&mut Cursor::new(output.stdout), &mut content, &task.marker_precompiled, task.output_precompiled.is_some()));
			} else {				
				try! (content.write(&output.stdout));
			};
			Ok(PreprocessResult::Success(content))
		} else {
			Ok(PreprocessResult::Failed(OutputInfo{
				status: output.status.code(),
				stdout: Vec::new(),
				stderr: output.stderr,
			}))
		}
	}

	// Compile preprocessed file.
	fn compile_prepare_step(&self, task: CompilationTask, preprocessed: MemStream) -> Result<CompileStep, Error> {
		let mut args = filter(&task.args, |arg:&Arg|->Option<String> {
			match arg {
				&Arg::Flag{ref scope, ref flag} => {
					match scope {
						&Scope::Compiler | &Scope::Shared => Some("/".to_string() + &flag),
						&Scope::Preprocessor if task.output_precompiled.is_some() => Some("/".to_string() + &flag),
						&Scope::Ignore | &Scope::Preprocessor => None
					}
				}
				&Arg::Param{ref scope, ref  flag, ref value} => {
					match scope {
						&Scope::Compiler | &Scope::Shared => Some("/".to_string() + &flag + &value),
						&Scope::Preprocessor if task.output_precompiled.is_some() => Some("/".to_string() + &flag + &value),
						&Scope::Ignore | &Scope::Preprocessor => None
					}
				}
				&Arg::Input{..} => None,
				&Arg::Output{..} => None
			}
		});
		args.push("/nologo".to_string());
		args.push("/T".to_string() + &task.language);
		match &task.input_precompiled {
			&Some(ref path) => {
				args.push("/Yu".to_string());
				args.push("/Fp".to_string() + &path.display().to_string());
			}
			&None => {}
		}
		if task.output_precompiled.is_some() {
			args.push("/Yc".to_string());
		}
		// Input data, stored in files.
		let mut inputs: Vec<PathBuf> = Vec::new();
		match &task.input_precompiled {
				&Some(ref path) => {inputs.push(path.clone());}
				&None => {}
			}
		// Output files.
		let mut outputs: Vec<PathBuf> = Vec::new();
		outputs.push(task.output_object.clone());
		match &task.output_precompiled {
			&Some(ref path) => {
				outputs.push(path.clone());
				args.push(join_flag("/Fp", path));
			}
			&None => {}
		}
		match &task.input_precompiled {
			&Some(ref path) => {args.push(join_flag("/Fp", path));}
			&None => {}
		}
		Ok(CompileStep::new(task, preprocessed, args, inputs, outputs))
	}

	fn compile_task(&self, task: CompileStep) -> Result<OutputInfo, Error> {
		// Input file path.
		let input_temp = TempFile::new_in(&self.temp_dir, ".i");
		try! (File::create(input_temp.path()).and_then(|mut s| task.preprocessed.copy(&mut s)));
		// Run compiler.
		let mut command = task.command.to_command();
		command
		.arg("/c")
		.args(&task.args)
		.arg(input_temp.path().to_str().unwrap())
		.arg(&join_flag("/Fo", &task.output_object));

		let temp_file = input_temp.path().file_name()
		.and_then(|o| o.to_str())
		.map(|o| o.as_bytes())
		.unwrap_or(b"");
		command.output().map(|o| OutputInfo {
			status: o.status.code(),
			stdout: prepare_output(temp_file, o.stdout),
			stderr: o.stderr,
		})
	}
}

fn prepare_output(line: &[u8], mut buffer: Vec<u8>) -> Vec<u8> {
	let mut begin = match (line.len() < buffer.len()) && buffer.starts_with(line) && is_eol(buffer[line.len()]) {
		true => line.len(),
		false => 0
	};
	while begin < buffer.len() && is_eol(buffer[begin]) {
		begin += 1;
	}
	buffer.split_off(begin)
}

fn is_eol(c: u8) -> bool {
	match c {
	    b'\r' | b'\n' => true,
	    _ => false,
	}
}

fn join_flag(flag: &str, path: &Path) -> String {
	flag.to_string() + &path.to_str().unwrap()
}