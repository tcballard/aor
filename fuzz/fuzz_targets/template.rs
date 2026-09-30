#![no_main]
use aor_tmpl::{Template,TemplateContext};
#[derive(TemplateContext)]struct Context{title:String,show:bool,items:Vec<String>}
libfuzzer_sys::fuzz_target!(|data:&[u8]|{
    if let Ok(source)=std::str::from_utf8(data){if let Ok(template)=Template::parse(source){let _=template.render(&Context{title:"<script>".into(),show:true,items:vec!["&<>".into()]});}}
});
