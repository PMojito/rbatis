use rdbc::Connection;
use serde::de;
use serde_json::Value;
use uuid::Uuid;

use crate::session::Session;
use crate::tx::propagation::Propagation;
use crate::tx::save_point_stack::SavePointStack;
use crate::tx::tx_stack::TxStack;
use crate::utils::rdbc_util;
use crate::decode::rdbc_driver_decoder::decode_result_set;
use log::{error, info, warn};
use crate::utils::rdbc_util::to_rdbc_values;
use serde_json::de::ParserNumber;

pub struct LocalSession {
    pub session_id: String,
    pub driver: String,
    pub tx_stack: TxStack,
    pub save_point_stack: SavePointStack,
    pub is_closed: bool,
    pub new_local_session: Option<Box<LocalSession>>,
    pub enable_log:bool,

    pub conn: Option<Box<dyn Connection>>,
}

impl LocalSession {
    pub fn new(id: &str, driver: &str) -> Self {
        let mut new_id = id.to_string();
        if new_id.is_empty() {
            new_id = Uuid::new_v4().to_string();
        }
       return Self {
            session_id: new_id,
            driver: driver.to_string(),
            tx_stack: TxStack::new(),
            save_point_stack: SavePointStack::new(),
            is_closed: false,
            new_local_session: None,
            enable_log: true,
            conn: None,
        }
    }

}

impl Session for LocalSession {
    fn id(&self) -> String {
        return Uuid::new_v4().to_string();
    }

    fn query<T>(&mut self, sql: &str, arg_array: &mut Vec<Value>) -> Result<T, String> where T: de::DeserializeOwned {
        if self.is_closed == true{
            return Err("[rbatis] session can not query a closed session!".to_string())
        }
        if self.new_local_session.is_some(){
            return self.new_local_session.as_mut().unwrap().query(sql,arg_array);
        }
        let params = to_rdbc_values(arg_array);
        if self.enable_log {
                info!("[rbatis] Query: ==>  {}: ", sql);
                info!("[rbatis]  Args: ==>  {}: ", rdbc_util::rdbc_vec_to_string(&params));
        }
        let (mut t_opt,_)= self.tx_stack.last();
        if t_opt.is_some(){
            let t= t_opt.unwrap();
            let result= t.query(sql,arg_array)?;
            return result;
        }else{
            let conn= self.conn.as_mut().unwrap();
            let create_result = conn.create(sql);
            if create_result.is_err() {
                return Result::Err("[rbatis] select fail:".to_string() + format!("{:?}", create_result.err().unwrap()).as_str());
            }
            let mut create_statement = create_result.unwrap();
            let exec_result = create_statement.execute_query(&params);
            if exec_result.is_err() {
                return Result::Err("[rbatis] select fail:".to_string() + format!("{:?}", exec_result.err().unwrap()).as_str());
            }
            let (result, decoded_num) = decode_result_set(exec_result.unwrap().as_mut());
            if self.enable_log {
                info!("[rbatis] Total: <==  {}", decoded_num.to_string().as_str());
            }
            return result;
        }
    }

    fn exec(&mut self, sql: &str, arg_array: &mut Vec<Value>) -> Result<u64, String> {
        if self.is_closed == true{
            return Err("[rbatis] session can not query a closed session!".to_string())
        }
        if self.new_local_session.is_some(){
            return self.new_local_session.as_mut().unwrap().query(sql,arg_array);
        }
        let params = to_rdbc_values(arg_array);
        if self.enable_log {
            info!("[rbatis] Query: ==>  {}: ", sql);
            info!("[rbatis]  Args: ==>  {}: ", rdbc_util::rdbc_vec_to_string(&params));
        }
        let (mut t_opt,_)= self.tx_stack.last();
        if t_opt.is_some(){
            let t= t_opt.unwrap();
            let result= t.query(sql,arg_array)?;
            return result;
        }else{
            let conn= self.conn.as_mut().unwrap();
            let create_result = conn.create(sql);
            if create_result.is_err() {
                return Result::Err("[rbatis] exec fail:".to_string()  + format!("{:?}", create_result.err().unwrap()).as_str());
            }
            let exec_result = create_result.unwrap().execute_update(&params);
            if exec_result.is_err() {
                return Result::Err("[rbatis] exec fail:".to_string()  + format!("{:?}", exec_result.err().unwrap()).as_str());
            }
            let affected_rows = exec_result.unwrap();
            if self.enable_log {
                info!("[rbatis] Affected: <== {}", affected_rows.to_string().as_str());
            }
            return Result::Ok(affected_rows);
        }
    }

    fn rollback(&mut self) -> Result<u64, String> {
        if self.is_closed == true{
            return Err("[rbatis] session can not query a closed session!".to_string())
        }
        let mut closec_num=0;
        if self.new_local_session.is_some(){
            let new_session=self.new_local_session.as_mut().unwrap();
            let r=new_session.rollback()?;
            new_session.close();
            closec_num+=r;
        }

        let (t_opt,p_opt)=self.tx_stack.pop();
        if t_opt.is_some() && p_opt.is_some(){
            let mut t =t_opt.unwrap();
            if self.propagation().is_some(){
                if self.propagation().as_ref().unwrap().eq(&Propagation::NESTED){
                    let point_opt=  self.save_point_stack.pop();
                    if point_opt.is_some(){
                        info!("[rbatis] [{}] exec ============ rollback",self.session_id.as_str());
                        let sql="rollback to ".to_string()+point_opt.unwrap().as_str();
                        let r=t.exec(sql.as_str(),&mut vec![])?;
                        closec_num+=r;
                        return Ok(closec_num);
                    }
                }
            }
            if self.tx_stack.len()==0{
                info!("[rbatis] [{}] exec ============ rollback",self.session_id.as_str());
                let r=t.rollback()?;
                closec_num+=r;
                return Ok(closec_num);
            }
        }
        return Ok(closec_num);
    }

    fn commit(&mut self) -> Result<u64, String> {
        unimplemented!()
    }

    fn begin(&mut self, propagation_type: Option<Propagation>) -> Result<u64, String> {
        if propagation_type.is_some() {
            match propagation_type.as_ref().unwrap() {
                Propagation::REQUIRED => {
                    if self.tx_stack.len() > 0 {}
                }
                Propagation::NOT_SUPPORTED => {
                    if self.tx_stack.len() > 0 {
                        //TODO stop old tx
                    }
                    self.new_local_session = Some(Box::new(LocalSession::new("", self.driver.as_str())));
                }
                _ => {}
            }
        }
        return Ok(0);
    }

    fn close(&mut self) {
        unimplemented!()
    }

    fn propagation(&self) -> Option<Propagation> {
        unimplemented!()
    }
}
